// crates/afterglow-web/web/src/engine/audio/pcm-ring.ts
var AUDIO_QUANTUM_FRAMES = 128;
var AUDIO_CHANNELS = 2;
var AUDIO_PCM_SAMPLES = AUDIO_QUANTUM_FRAMES * AUDIO_CHANNELS;
var AUDIO_RING_HEADER_WORDS = 3;
var AUDIO_RING_PAYLOAD_WORDS = 1 + AUDIO_PCM_SAMPLES;
var AUDIO_RING_FRAME_WORDS = 1 + AUDIO_RING_PAYLOAD_WORDS;
var AUDIO_RING_FRAME_BYTES = AUDIO_RING_FRAME_WORDS * 4;
var AUDIO_RING_TELEMETRY_WORDS = 12;
function audioPcmRingBytes(slotCapacity) {
  if (!Number.isInteger(slotCapacity) || slotCapacity < 2 || slotCapacity > 8)
    throw new RangeError("audio PCM ring capacity must be between 2 and 8 quanta");
  return (AUDIO_RING_HEADER_WORDS + slotCapacity * AUDIO_RING_FRAME_WORDS + AUDIO_RING_TELEMETRY_WORDS) * 4;
}
function createAudioPcmRing(slotCapacity) {
  const memory = new SharedArrayBuffer(audioPcmRingBytes(slotCapacity));
  const header = new Int32Array(memory, 0, AUDIO_RING_HEADER_WORDS);
  Atomics.store(header, 0 /* CapacityBytes */, slotCapacity * AUDIO_RING_FRAME_BYTES);
  return memory;
}
function audioPcmRingTelemetry(memory) {
  const header = new Int32Array(memory, 0, AUDIO_RING_HEADER_WORDS);
  const capacityBytes = Atomics.load(header, 0 /* CapacityBytes */) >>> 0;
  return new Int32Array(memory, AUDIO_RING_HEADER_WORDS * 4 + capacityBytes, AUDIO_RING_TELEMETRY_WORDS);
}

// crates/afterglow-web/web/src/workers/codec.ts
function encodeVarint(n) {
  const b = [];
  do {
    let x = n & 127;
    n = Math.floor(n / 128);
    if (n)
      x |= 128;
    b.push(x);
  } while (n);
  return b;
}
function decodeVarint(bytes, off) {
  let r = 0;
  for (let shift = 0;shift < 56; shift += 7) {
    if (off >= bytes.length)
      throw new Error("postcard varint truncated");
    const b = bytes[off++];
    r += (b & 127) * 2 ** shift;
    if (!(b & 128))
      return [r, off];
  }
  throw new Error("postcard varint overflows");
}
function decodeZigzag(bytes, off) {
  const [zz, o] = decodeVarint(bytes, off);
  return [zz & 1 ? -(zz + 1) / 2 : zz / 2, o];
}
function concat(...arrs) {
  const out = new Uint8Array(arrs.reduce((s, a) => s + a.length, 0));
  let o = 0;
  for (const a of arrs) {
    out.set(a, o);
    o += a.length;
  }
  return out;
}
function encodeU32(n) {
  return new Uint8Array(encodeVarint(n));
}
function decodeU32(bytes, off) {
  return decodeVarint(bytes, off);
}
function decodeI32(bytes, off) {
  return decodeZigzag(bytes, off);
}
function encodeF32(x) {
  const b = new Uint8Array(4);
  new DataView(b.buffer).setFloat32(0, x, true);
  return b;
}
function encodeBool(b) {
  return new Uint8Array([b ? 1 : 0]);
}
function encodeBytes(b) {
  return concat(encodeVarint(b.length), b);
}
function decodeF64Vec(bytes, off) {
  const [n, o] = decodeVarint(bytes, off);
  const end = o + n * 8;
  if (end > bytes.length)
    throw new Error("postcard f64 vec truncated");
  const out = new Float64Array(n);
  const dv = new DataView(bytes.buffer, bytes.byteOffset + o, n * 8);
  for (let i = 0;i < n; i++)
    out[i] = dv.getFloat64(i * 8, true);
  return [out, end];
}
function unwrapResponse(bytes) {
  const [variant, off] = decodeVarint(bytes, 0);
  if (variant === 0) {
    const [plen, poff] = decodeVarint(bytes, off);
    if (poff + plen > bytes.length)
      throw new Error("RPC response truncated");
    return bytes.subarray(poff, poff + plen);
  }
  const [method, moff] = decodeVarint(bytes, off);
  const [mlen, eoff] = decodeVarint(bytes, moff);
  if (eoff + mlen > bytes.length)
    throw new Error("RPC error truncated");
  const msg = new TextDecoder().decode(Uint8Array.from(bytes.subarray(eoff, eoff + mlen)));
  throw new Error(`RPC ${variant === 1 ? "server" : "decode"} error (method ${method}): ${msg}`);
}

// crates/afterglow-web/web/src/workers/rpc.ts
var TIMEOUT_MS = 5000;

class Rpc {
  static async create({ mainWasmUrl, workerJsUrl, workerWasmUrl, timeoutMs, workerInit = null }) {
    const memory = new WebAssembly.Memory({ shared: true, initial: 256, maximum: 1024 });
    const worker = new Worker(workerJsUrl, { type: "module" });
    let rpc = null;
    try {
      const { exports: wasm } = await WebAssembly.instantiate(await WebAssembly.compile(await (await fetch(mainWasmUrl)).arrayBuffer()), { env: { memory, notify_worker: () => worker.postMessage("wake") } });
      wasm.init_ring_buffers();
      rpc = new Rpc(wasm, memory, worker, { timeoutMs });
      worker.postMessage({
        type: "init",
        sab: memory.buffer,
        reqBase: wasm.get_request_ptr(),
        respBase: wasm.get_response_ptr(),
        bufSize: wasm.get_buffer_size(),
        wasmUrl: workerWasmUrl,
        workerInit
      });
      await rpc._initPromise;
      worker.postMessage({ type: "run" });
      return rpc;
    } catch (e) {
      if (rpc)
        rpc.terminate();
      else
        worker.terminate();
      throw e;
    }
  }
  constructor(wasm, memory, worker, opts = {}) {
    this.w = wasm;
    this.mem = memory;
    this.worker = worker;
    this.scratch = wasm.get_scratch_ptr();
    this.scratchLen = wasm.get_scratch_size();
    this.pending = null;
    this._resolve = null;
    this._reject = null;
    this._fatal = null;
    this._terminated = false;
    this.timeoutMs = opts.timeoutMs ?? TIMEOUT_MS;
    this._initPromise = new Promise((res, rej) => {
      this._resolve = res;
      this._reject = rej;
    });
    this._initTimer = setTimeout(() => this._fail(new Error("worker init timeout")), this.timeoutMs);
    worker.onmessage = (e) => this._onmsg(e.data);
    worker.onerror = (e) => this._fail(new Error("worker: " + (e && e.message || e)));
  }
  _onmsg(d) {
    if (this._fatal)
      return;
    if (d && d.type === "ready") {
      clearTimeout(this._initTimer);
      const r = this._resolve;
      this._resolve = this._reject = null;
      if (r)
        r();
      return;
    }
    if (d && d.type === "error") {
      this._fail(this._reject ? new Error("worker init: " + (d.message || "error")) : new Error(d.message || "worker error"));
      return;
    }
    if (this.pending)
      this._readResponse();
  }
  async call(method, args) {
    if (this._fatal)
      throw this._fatal;
    if (this.pending)
      throw new Error("RPC busy: one in-flight call at a time");
    const len = 4 + args.length;
    if (len > this.scratchLen)
      throw new Error("request too large for scratch");
    const view = new Uint8Array(this.mem.buffer, this.scratch, len);
    view[0] = method & 255;
    view[1] = method >>> 8 & 255;
    view[2] = method >>> 16 & 255;
    view[3] = method >>> 24 & 255;
    view.set(args, 4);
    if (this.w.write_frame(this.scratch, len) !== 0)
      throw new Error("write_frame failed (ring full)");
    return new Promise((resolve, reject) => {
      this.pending = { resolve, reject };
      this.pending.timer = setTimeout(() => this._fail(new Error("RPC timeout")), this.timeoutMs);
    });
  }
  _readResponse() {
    const n = this.w.read_response(this.scratch, this.scratchLen);
    const p = this.pending;
    this.pending = null;
    if (p)
      clearTimeout(p.timer);
    if (!p)
      return;
    if (n < 0) {
      p.reject(new Error("read_response returned " + n));
      return;
    }
    try {
      p.resolve(unwrapResponse(new Uint8Array(this.mem.buffer, this.scratch, n)));
    } catch (e) {
      p.reject(e);
    }
  }
  _fail(err) {
    if (this._fatal)
      return;
    this._fatal = err;
    clearTimeout(this._initTimer);
    if (this._reject) {
      const r = this._reject;
      this._resolve = this._reject = null;
      r(err);
    }
    if (this.pending) {
      const p = this.pending;
      this.pending = null;
      clearTimeout(p.timer);
      p.reject(err);
    }
  }
  terminate() {
    this._fail(new Error("terminated"));
    if (!this._terminated) {
      this._terminated = true;
      this.worker.terminate();
    }
  }
}

// crates/afterglow-web/web/src/workers/engineaudioservice.client.ts
class EngineAudioServiceClient {
  rpc;
  closed = false;
  static async spawn(opts = {}) {
    const rpc = await Rpc.create({
      mainWasmUrl: opts.mainWasmUrl ?? "afterglow_rpc.wasm",
      workerJsUrl: opts.workerJsUrl ?? "worker.js",
      workerWasmUrl: opts.workerWasmUrl ?? "engineaudioservice.wasm",
      timeoutMs: opts.timeoutMs
    });
    return new EngineAudioServiceClient(rpc);
  }
  constructor(rpc) {
    this.rpc = rpc;
  }
  close() {
    if (this.closed)
      return;
    this.closed = true;
    this.rpc.terminate?.();
  }
  async configure(targetQuanta, triangles, reflectionRays, reflectionBounces, reflectionDurationMs, reflectionOrder) {
    const args = concat(encodeU32(targetQuanta), encodeU32(triangles), encodeU32(reflectionRays), encodeU32(reflectionBounces), encodeU32(reflectionDurationMs), encodeU32(reflectionOrder));
    const resp = await this.rpc.call(0, args);
    return decodeI32(resp, 0)[0];
  }
  async start() {
    const args = new Uint8Array;
    const resp = await this.rpc.call(1, args);
    return decodeI32(resp, 0)[0];
  }
  async stop() {
    const args = new Uint8Array;
    const resp = await this.rpc.call(2, args);
    return decodeI32(resp, 0)[0];
  }
  async updateMotion(phase) {
    const args = encodeF32(phase);
    const resp = await this.rpc.call(3, args);
    return decodeI32(resp, 0)[0];
  }
  async runSimulation() {
    const args = new Uint8Array;
    const resp = await this.rpc.call(4, args);
    return decodeI32(resp, 0)[0];
  }
  async stats() {
    const args = new Uint8Array;
    const resp = await this.rpc.call(5, args);
    return decodeF64Vec(resp, 0)[0];
  }
  async shutdown() {
    const args = new Uint8Array;
    const resp = await this.rpc.call(6, args);
    return decodeI32(resp, 0)[0];
  }
  async spawn2d(sound) {
    const args = encodeU32(sound);
    const resp = await this.rpc.call(7, args);
    return decodeU32(resp, 0)[0];
  }
  async spawnAt(sound, x, y, z) {
    const args = concat(encodeU32(sound), encodeF32(x), encodeF32(y), encodeF32(z));
    const resp = await this.rpc.call(8, args);
    return decodeU32(resp, 0)[0];
  }
  async spawnAttached(sound, entity) {
    const args = concat(encodeU32(sound), encodeU32(entity));
    const resp = await this.rpc.call(9, args);
    return decodeU32(resp, 0)[0];
  }
  async spawnSpatialOnly(sound, x, y, z) {
    const args = concat(encodeU32(sound), encodeF32(x), encodeF32(y), encodeF32(z));
    const resp = await this.rpc.call(10, args);
    return decodeU32(resp, 0)[0];
  }
  async spawnListenerRelative(sound, x, y, z) {
    const args = concat(encodeU32(sound), encodeF32(x), encodeF32(y), encodeF32(z));
    const resp = await this.rpc.call(11, args);
    return decodeU32(resp, 0)[0];
  }
  async crossfade(from, to, seconds) {
    const args = concat(encodeU32(from), encodeU32(to), encodeF32(seconds));
    const resp = await this.rpc.call(12, args);
    return decodeI32(resp, 0)[0];
  }
  async crossfadeTo(from, sound, seconds) {
    const args = concat(encodeU32(from), encodeU32(sound), encodeF32(seconds));
    const resp = await this.rpc.call(13, args);
    return decodeU32(resp, 0)[0];
  }
  async setVoiceVolume(handle, volume, seconds) {
    const args = concat(encodeU32(handle), encodeF32(volume), encodeF32(seconds));
    const resp = await this.rpc.call(14, args);
    return decodeI32(resp, 0)[0];
  }
  async pauseVoice(handle, seconds) {
    const args = concat(encodeU32(handle), encodeF32(seconds));
    const resp = await this.rpc.call(15, args);
    return decodeI32(resp, 0)[0];
  }
  async resumeVoice(handle, seconds) {
    const args = concat(encodeU32(handle), encodeF32(seconds));
    const resp = await this.rpc.call(16, args);
    return decodeI32(resp, 0)[0];
  }
  async stopVoice(handle, seconds) {
    const args = concat(encodeU32(handle), encodeF32(seconds));
    const resp = await this.rpc.call(17, args);
    return decodeI32(resp, 0)[0];
  }
  async loadWav(data, looped) {
    const args = concat(encodeBytes(data), encodeBool(looped));
    const resp = await this.rpc.call(18, args);
    return decodeU32(resp, 0)[0];
  }
  async unloadSound(handle) {
    const args = encodeU32(handle);
    const resp = await this.rpc.call(19, args);
    return decodeI32(resp, 0)[0];
  }
  async beginWavUpload(totalBytes, looped) {
    const args = concat(encodeU32(totalBytes), encodeBool(looped));
    const resp = await this.rpc.call(20, args);
    return decodeI32(resp, 0)[0];
  }
  async appendWavUpload(data) {
    const args = encodeBytes(data);
    const resp = await this.rpc.call(21, args);
    return decodeI32(resp, 0)[0];
  }
  async finishWavUpload() {
    const args = new Uint8Array;
    const resp = await this.rpc.call(22, args);
    return decodeU32(resp, 0)[0];
  }
  async beginAcousticSceneUpload(totalBytes) {
    const args = encodeU32(totalBytes);
    const resp = await this.rpc.call(23, args);
    return decodeI32(resp, 0)[0];
  }
  async appendAcousticSceneUpload(data) {
    const args = encodeBytes(data);
    const resp = await this.rpc.call(24, args);
    return decodeI32(resp, 0)[0];
  }
  async finishAcousticSceneUpload() {
    const args = new Uint8Array;
    const resp = await this.rpc.call(25, args);
    return decodeI32(resp, 0)[0];
  }
}

// crates/afterglow-web/web/src/demos/audio-worklet/main.ts
var UPLOAD_CHUNK_BYTES = 512 * 1024;
var REAL_SOUND_NAMES = ["abcd.wav", "counting.wav", "impulse.wav", "ozymandias.wav", "pinknoise.wav"];
var output = document.getElementById("output");
var startButton = document.getElementById("start");
var stopButton = document.getElementById("stop");
if (!(output instanceof HTMLElement) || !(startButton instanceof HTMLButtonElement) || !(stopButton instanceof HTMLButtonElement))
  throw new Error("missing audio diagnostic controls");
var context = null;
var sink = null;
var client = null;
var timer = 0;
function depthFromUrl() {
  const value = Number(new URLSearchParams(location.search).get("quanta") ?? 8);
  return Number.isInteger(value) && value >= 2 && value <= 8 ? value : 8;
}
function status(code, operation) {
  if (code !== 0)
    throw new Error(`${operation} failed with status ${code}`);
}
async function fetchBytes(url) {
  const response = await fetch(url);
  if (!response.ok)
    throw new Error(`fetch ${url} failed with ${response.status}`);
  return new Uint8Array(await response.arrayBuffer());
}
async function uploadWav(client2, bytes) {
  status(await client2.beginWavUpload(bytes.byteLength, true), "begin WAV upload");
  for (let offset = 0;offset < bytes.byteLength; offset += UPLOAD_CHUNK_BYTES)
    status(await client2.appendWavUpload(bytes.subarray(offset, offset + UPLOAD_CHUNK_BYTES)), "append WAV upload");
  const handle = await client2.finishWavUpload();
  if (handle === 0)
    throw new Error("finish WAV upload rejected the sound");
  return handle;
}
async function uploadAcousticScene(client2, bytes) {
  status(await client2.beginAcousticSceneUpload(bytes.byteLength), "begin acoustic scene upload");
  for (let offset = 0;offset < bytes.byteLength; offset += UPLOAD_CHUNK_BYTES)
    status(await client2.appendAcousticSceneUpload(bytes.subarray(offset, offset + UPLOAD_CHUNK_BYTES)), "append acoustic scene upload");
  status(await client2.finishAcousticSceneUpload(), "finish acoustic scene upload");
}
function diagnosticStereoWav() {
  const frames = 4800;
  const channels = 2;
  const dataBytes = frames * channels * 2;
  const bytes = new Uint8Array(44 + dataBytes);
  const view = new DataView(bytes.buffer);
  const text = (offset, value) => {
    for (let index = 0;index < value.length; ++index)
      bytes[offset + index] = value.charCodeAt(index);
  };
  text(0, "RIFF");
  view.setUint32(4, 36 + dataBytes, true);
  text(8, "WAVE");
  text(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, channels, true);
  view.setUint32(24, 48000, true);
  view.setUint32(28, 48000 * channels * 2, true);
  view.setUint16(32, channels * 2, true);
  view.setUint16(34, 16, true);
  text(36, "data");
  view.setUint32(40, dataBytes, true);
  for (let frame = 0;frame < frames; ++frame) {
    view.setInt16(44 + frame * 4, Math.sin(frame * Math.PI * 2 * 220 / 48000) * 8192, true);
    view.setInt16(46 + frame * 4, Math.sin(frame * Math.PI * 2 * 330 / 48000) * 8192, true);
  }
  return bytes;
}
startButton.addEventListener("click", async () => {
  if (context !== null)
    return;
  try {
    const depth = depthFromUrl();
    const search = new URLSearchParams(location.search);
    const useSteam = search.has("steam");
    const useRealSounds = search.has("realSounds");
    const acousticScene = search.get("acousticScene");
    const pcmMemory = createAudioPcmRing(depth);
    const ringHeader = new Int32Array(pcmMemory, 0, 3);
    const telemetry = audioPcmRingTelemetry(pcmMemory);
    const rpc = await Rpc.create({
      mainWasmUrl: "afterglow_rpc.wasm",
      workerJsUrl: "audio-worker.js",
      workerWasmUrl: "engineaudioservice.wasm",
      timeoutMs: 1e4,
      workerInit: {
        pcmMemory,
        emscriptenModuleUrl: useSteam ? new URL("engine-audio-rpc.js", location.href).href : undefined
      }
    });
    client = new EngineAudioServiceClient(rpc);
    status(await client.configure(depth, 1e4, 512, 2, 500, 0), "configure");
    let sourceX = -1.5;
    let sourceY = 0;
    let sourceZ = -1;
    if (acousticScene !== null) {
      const sceneBytes = await fetchBytes(`real-audio/${acousticScene}.acoustic.bin`);
      if (sceneBytes.byteLength < 72)
        throw new Error("truncated acoustic scene header");
      const header = new DataView(sceneBytes.buffer, sceneBytes.byteOffset, 72);
      sourceX = header.getFloat32(60, true);
      sourceY = header.getFloat32(64, true);
      sourceZ = header.getFloat32(68, true);
      await uploadAcousticScene(client, sceneBytes);
    }
    const sounds = [];
    if (useRealSounds) {
      for (const name of REAL_SOUND_NAMES)
        sounds.push(await uploadWav(client, await fetchBytes(`real-audio/${name}`)));
    } else {
      const residentSound = await client.loadWav(diagnosticStereoWav(), true);
      if (residentSound === 0)
        throw new Error("resident WAV was not admitted");
      for (let index = 0;index < REAL_SOUND_NAMES.length; ++index)
        sounds.push(residentSound);
    }
    status(await client.runSimulation(), "simulation");
    status(await client.start(), "start");
    for (let index = 0;index < 4; ++index) {
      const voice = await client.spawnAt(sounds[index], sourceX + index * 0.12, sourceY, sourceZ);
      if (voice === 0)
        throw new Error(`physical voice ${index} was not admitted`);
    }
    const firstDry = await client.spawn2d(sounds[4]);
    if (firstDry === 0)
      throw new Error("resident dry voice was not admitted");
    const secondDry = await client.crossfadeTo(firstDry, sounds[1], 0.25);
    if (secondDry === 0)
      throw new Error("diagnostic crossfade was not admitted");
    status(await client.setVoiceVolume(secondDry, 0.65, 0.25), "volume ramp");
    await waitForPcmDepth(ringHeader, depth, 5000);
    context = new AudioContext({ sampleRate: 48000, latencyHint: "interactive" });
    await context.audioWorklet.addModule("engine/audio-worklet.js");
    sink = new AudioWorkletNode(context, "afterglow-engine-audio-sink", {
      numberOfInputs: 0,
      numberOfOutputs: 1,
      outputChannelCount: [2],
      processorOptions: { memory: pcmMemory, masterGain: 0.35 }
    });
    sink.connect(context.destination);
    await context.resume();
    Atomics.store(telemetry, 11 /* Armed */, 1);
    timer = window.setInterval(async () => {
      if (client === null)
        return;
      const worker = await client.stats();
      const write = Atomics.load(ringHeader, 1 /* WriteBytes */) >>> 0;
      const read = Atomics.load(ringHeader, 2 /* ReadBytes */) >>> 0;
      const rendered = Atomics.load(telemetry, 4 /* Rendered */);
      output.textContent = JSON.stringify({
        backend: useSteam ? "unified-rust-rpc-steam-audio-hybrid" : "unified-rust-rpc-synthetic-gate",
        soundSet: useRealSounds ? "Steam Audio SDK real speech/noise/impulse set" : "generated diagnostic WAV",
        acousticScene: acousticScene ?? "generated room",
        targetQuanta: depth,
        ringDepth: Math.floor((write - read >>> 0) / AUDIO_RING_FRAME_BYTES),
        rendered,
        callbacks: Atomics.load(telemetry, 3 /* Callbacks */),
        underruns: Atomics.load(telemetry, 1 /* Underruns */),
        sequenceErrors: Atomics.load(telemetry, 2 /* SequenceErrors */),
        wakeHits: Atomics.load(telemetry, 6 /* WakeHits */),
        wakeMisses: Atomics.load(telemetry, 7 /* WakeMisses */),
        pumpMeanMs: rendered === 0 ? 0 : Atomics.load(telemetry, 8 /* PumpMicros */) / rendered / 1000,
        pumpMaxMs: Atomics.load(telemetry, 9 /* PumpMaxMicros */) / 1000,
        pumpOverBudget: Atomics.load(telemetry, 10 /* PumpOverBudget */),
        fatal: Atomics.load(telemetry, 5 /* Fatal */),
        sampleClock: worker[0],
        workerRendered: worker[1],
        simulationUpdates: worker[2],
        outputEnergy: worker[3],
        outputPeak: worker[4],
        lastImpulseSample: worker[5],
        voices: worker[7],
        reflectionVoices: worker[8],
        activeSpatialVoices: worker[15],
        activeReflectionVoices: worker[16],
        activeVoices: worker[17],
        activeWorldPhysicalVoices: worker[18],
        rejectedVoiceCapacity: worker[19],
        rejectedPhysicalCapacity: worker[20],
        staleVoiceHandles: worker[21],
        completedVoiceFades: worker[22],
        loadedResidentSounds: worker[23],
        residentSoundBytes: worker[24],
        acousticVertices: worker[25],
        acousticTriangles: worker[26],
        acousticSceneBytes: worker[27]
      }, null, 2);
    }, 250);
    startButton.disabled = true;
    stopButton.disabled = false;
  } catch (error) {
    output.textContent = `FATAL AUDIO: ${String(error.message ?? error)}`;
    await dispose();
  }
});
stopButton.addEventListener("click", () => {
  dispose();
});
window.addEventListener("pagehide", () => {
  dispose();
}, { once: true });
async function waitForPcmDepth(header, target, timeoutMs) {
  const deadline = performance.now() + timeoutMs;
  while (performance.now() < deadline) {
    const write = Atomics.load(header, 1 /* WriteBytes */) >>> 0;
    const read = Atomics.load(header, 2 /* ReadBytes */) >>> 0;
    if (Math.floor((write - read >>> 0) / AUDIO_RING_FRAME_BYTES) >= target)
      return;
    await new Promise((resolve) => setTimeout(resolve, 1));
  }
  throw new Error("EngineAudio worker did not prefill the final PCM ring");
}
async function dispose() {
  if (timer !== 0) {
    clearInterval(timer);
    timer = 0;
  }
  sink?.disconnect();
  sink = null;
  if (client !== null) {
    try {
      await client.stop();
      await client.shutdown();
    } catch {}
    client.close();
    client = null;
  }
  if (context !== null) {
    await context.close();
    context = null;
  }
  startButton.disabled = false;
  stopButton.disabled = true;
}
