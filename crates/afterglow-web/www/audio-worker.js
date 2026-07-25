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
function audioPcmRingTelemetry(memory) {
  const header = new Int32Array(memory, 0, AUDIO_RING_HEADER_WORDS);
  const capacityBytes = Atomics.load(header, 0 /* CapacityBytes */) >>> 0;
  return new Int32Array(memory, AUDIO_RING_HEADER_WORDS * 4 + capacityBytes, AUDIO_RING_TELEMETRY_WORDS);
}

class AudioPcmRingView {
  memory;
  header;
  words;
  samples;
  telemetry;
  slotCapacity;
  capacityBytes;
  constructor(memory) {
    this.memory = memory;
    this.header = new Int32Array(memory, 0, AUDIO_RING_HEADER_WORDS);
    this.capacityBytes = Atomics.load(this.header, 0 /* CapacityBytes */) >>> 0;
    if (this.capacityBytes % AUDIO_RING_FRAME_BYTES !== 0)
      throw new RangeError("invalid audio PCM RingBuffer byte capacity");
    this.slotCapacity = this.capacityBytes / AUDIO_RING_FRAME_BYTES;
    if (memory.byteLength !== audioPcmRingBytes(this.slotCapacity))
      throw new RangeError("invalid audio PCM ring storage");
    this.words = new Int32Array(memory);
    this.samples = new Float32Array(memory);
    this.telemetry = audioPcmRingTelemetry(memory);
  }
  frameWord(byteOffset) {
    return AUDIO_RING_HEADER_WORDS + byteOffset % this.capacityBytes / 4;
  }
}

class AudioPcmRingWriter extends AudioPcmRingView {
  tryWrite(interleaved) {
    if (interleaved.length !== AUDIO_PCM_SAMPLES) {
      Atomics.store(this.telemetry, 5 /* Fatal */, 1);
      return false;
    }
    const write = Atomics.load(this.header, 1 /* WriteBytes */) >>> 0;
    const read = Atomics.load(this.header, 2 /* ReadBytes */) >>> 0;
    if (write - read >>> 0 > this.capacityBytes - AUDIO_RING_FRAME_BYTES)
      return false;
    const frame = this.frameWord(write);
    const sequence = Math.floor(write / AUDIO_RING_FRAME_BYTES) >>> 0;
    this.words[frame] = AUDIO_RING_PAYLOAD_WORDS * 4;
    this.words[frame + 1] = sequence | 0;
    const sampleBase = frame + 2;
    for (let index = 0;index < AUDIO_PCM_SAMPLES; ++index)
      this.samples[sampleBase + index] = interleaved[index];
    Atomics.store(this.header, 1 /* WriteBytes */, write + AUDIO_RING_FRAME_BYTES | 0);
    Atomics.add(this.telemetry, 4 /* Rendered */, 1);
    return true;
  }
  get depth() {
    const write = Atomics.load(this.header, 1 /* WriteBytes */) >>> 0;
    const read = Atomics.load(this.header, 2 /* ReadBytes */) >>> 0;
    return Math.floor((write - read >>> 0) / AUDIO_RING_FRAME_BYTES);
  }
}

// crates/afterglow-web/web/src/workers/ring-buf.ts
var U32 = 4;
var HEADER = 12;
function rdU32(u8, off, cap) {
  return (u8[off % cap] | u8[(off + 1) % cap] << 8 | u8[(off + 2) % cap] << 16 | u8[(off + 3) % cap] << 24) >>> 0;
}
function wrU32(u8, off, cap, val) {
  u8[off % cap] = val & 255;
  u8[(off + 1) % cap] = val >>> 8 & 255;
  u8[(off + 2) % cap] = val >>> 16 & 255;
  u8[(off + 3) % cap] = val >>> 24 & 255;
}
function xfer(u8, off, cap, buf, len, mode) {
  const o = off % cap, first = Math.min(len, cap - o);
  if (mode === "rd") {
    buf.set(u8.subarray(o, o + first), 0);
    if (first < len)
      buf.set(u8.subarray(0, len - first), first);
  } else {
    u8.set(buf.subarray(0, first), o);
    if (first < len)
      u8.set(buf.subarray(first), 0);
  }
}

// crates/afterglow-web/web/src/workers/audio-worker.ts
var initMessage = null;
var wasm = null;
var wasmMemory = null;
var wasmBuffer = null;
var inputPtr = 0;
var inputSize = 0;
var outputPtr = 0;
var outputSize = 0;
var pcmWriter = null;
var pcmTelemetry = null;
var pcmQuantum = null;
var state = "init";
self.onmessage = (event) => {
  const message = event.data;
  if (state === "init" && typeof message === "object" && message.type === "init") {
    initialize(message);
    return;
  }
  if (state === "ready" && typeof message === "object" && message.type === "run") {
    state = "running";
    run();
  }
};
async function initialize(message) {
  try {
    if (!(message.workerInit?.pcmMemory instanceof SharedArrayBuffer))
      throw new Error("audio worker requires PCM SharedArrayBuffer");
    initMessage = message;
    if (message.workerInit.emscriptenModuleUrl !== undefined) {
      const moduleUrl = message.workerInit.emscriptenModuleUrl;
      const imported = await import(moduleUrl);
      const emscripten = await imported.default({
        locateFile: (path) => new URL(path, moduleUrl).href
      });
      const heap = emscripten.HEAPU8;
      wasmBuffer = heap.buffer;
      wasm = {
        afterglow_wasm_init: emscripten._afterglow_wasm_init,
        afterglow_wasm_serve_frame: emscripten._afterglow_wasm_serve_frame,
        afterglow_wasm_input_ptr: emscripten._afterglow_wasm_input_ptr,
        afterglow_wasm_input_size: emscripten._afterglow_wasm_input_size,
        afterglow_wasm_output_ptr: emscripten._afterglow_wasm_output_ptr,
        afterglow_wasm_output_size: emscripten._afterglow_wasm_output_size,
        afterglow_audio_pump: emscripten._afterglow_audio_pump,
        afterglow_audio_pcm_ptr: emscripten._afterglow_audio_pcm_ptr,
        afterglow_audio_pcm_samples: emscripten._afterglow_audio_pcm_samples,
        afterglow_audio_simulate_motion: emscripten._afterglow_audio_simulate_motion
      };
    } else {
      wasmMemory = new WebAssembly.Memory({ shared: true, initial: 256, maximum: 1024 });
      const imports = { env: {
        memory: wasmMemory,
        notify_worker: () => {},
        performance_now: () => performance.now()
      } };
      const module = await WebAssembly.compile(await (await fetch(message.wasmUrl)).arrayBuffer());
      wasm = (await WebAssembly.instantiate(module, imports)).exports;
      wasmBuffer = wasmMemory.buffer;
    }
    wasm.afterglow_wasm_init();
    inputPtr = wasm.afterglow_wasm_input_ptr();
    inputSize = wasm.afterglow_wasm_input_size();
    outputPtr = wasm.afterglow_wasm_output_ptr();
    outputSize = wasm.afterglow_wasm_output_size();
    pcmWriter = new AudioPcmRingWriter(message.workerInit.pcmMemory);
    pcmTelemetry = pcmWriter.telemetry;
    refreshPcmView();
    state = "ready";
    self.postMessage({ type: "ready" });
  } catch (error) {
    self.postMessage({ type: "error", message: String(error.message ?? error) });
  }
}
function refreshPcmView() {
  if (wasm === null || wasmBuffer === null)
    return;
  const pointer = wasm.afterglow_audio_pcm_ptr();
  const samples = wasm.afterglow_audio_pcm_samples();
  if (pointer === 0 || samples !== 256)
    throw new Error("invalid Rust audio PCM export");
  pcmQuantum = new Float32Array(wasmBuffer, pointer, samples);
}
function run() {
  if (initMessage === null || wasm === null || wasmBuffer === null || pcmWriter === null || pcmTelemetry === null || pcmQuantum === null)
    return;
  const message = initMessage;
  const reqCap = Atomics.load(new Uint32Array(message.sab, message.reqBase, 1), 0) >>> 0;
  const respCap = Atomics.load(new Uint32Array(message.sab, message.respBase, 1), 0) >>> 0;
  if (reqCap !== message.bufSize - HEADER || respCap !== message.bufSize - HEADER) {
    fail("bad audio RPC ring capacity");
    return;
  }
  const reqWrite = new Int32Array(message.sab, message.reqBase + U32, 1);
  const reqRead = new Int32Array(message.sab, message.reqBase + 2 * U32, 1);
  const reqData = new Uint8Array(message.sab, message.reqBase + HEADER, reqCap);
  const respWrite = new Int32Array(message.sab, message.respBase + U32, 1);
  const respRead = new Int32Array(message.sab, message.respBase + 2 * U32, 1);
  const respData = new Uint8Array(message.sab, message.respBase + HEADER, respCap);
  const input = new Uint8Array(wasmBuffer, inputPtr, inputSize);
  const output = new Uint8Array(wasmBuffer, outputPtr, outputSize);
  let nextSimulationMs = performance.now() + 1000;
  let simulationPhase = 0;
  for (;; ) {
    let serviced = false;
    let attemptedPump = false;
    let pumpResult = 0;
    const requestWrite = Atomics.load(reqWrite, 0) >>> 0;
    const requestRead = Atomics.load(reqRead, 0) >>> 0;
    const used = requestWrite - requestRead >>> 0;
    if (used !== 0) {
      if (used > reqCap || used < U32) {
        fail("corrupt audio RPC request ring");
        return;
      }
      const offset = requestRead % reqCap;
      const payloadLength = rdU32(reqData, offset, reqCap);
      const frameLength = U32 + payloadLength;
      if (payloadLength < U32 || frameLength > used || frameLength > reqCap) {
        fail("corrupt audio RPC request frame");
        return;
      }
      const method = rdU32(reqData, offset + U32, reqCap) >>> 0;
      const argsLength = payloadLength - U32;
      if (argsLength > inputSize) {
        fail("audio RPC request exceeds scratch");
        return;
      }
      xfer(reqData, offset + 2 * U32, reqCap, input, argsLength, "rd");
      Atomics.store(reqRead, 0, requestRead + frameLength | 0);
      const responseLength = wasm.afterglow_wasm_serve_frame(method, inputPtr, argsLength, outputPtr, Math.min(outputSize, respCap - U32));
      if (responseLength < 0 || responseLength > outputSize) {
        fail("audio RPC service failed");
        return;
      }
      const responseWrite = Atomics.load(respWrite, 0) >>> 0;
      const responseRead = Atomics.load(respRead, 0) >>> 0;
      const responseUsed = responseWrite - responseRead >>> 0;
      const responseFrameLength = U32 + responseLength;
      if (responseFrameLength > respCap - responseUsed) {
        fail("audio RPC response ring full");
        return;
      }
      const responseOffset = responseWrite % respCap;
      wrU32(respData, responseOffset, respCap, responseLength);
      xfer(respData, responseOffset + U32, respCap, output, responseLength, "wr");
      Atomics.store(respWrite, 0, responseWrite + responseFrameLength | 0);
      if (method === 0)
        refreshPcmView();
      self.postMessage("wake");
      serviced = true;
    }
    if (pcmWriter.depth < pcmWriter.slotCapacity) {
      attemptedPump = true;
      const pumpStarted = performance.now();
      pumpResult = wasm.afterglow_audio_pump();
      const pumpMicros = Math.max(0, Math.round((performance.now() - pumpStarted) * 1000));
      Atomics.add(pcmTelemetry, 8 /* PumpMicros */, pumpMicros);
      if (pumpMicros > 2667)
        Atomics.add(pcmTelemetry, 10 /* PumpOverBudget */, 1);
      let priorMax = Atomics.load(pcmTelemetry, 9 /* PumpMaxMicros */);
      while (pumpMicros > priorMax) {
        const observed = Atomics.compareExchange(pcmTelemetry, 9 /* PumpMaxMicros */, priorMax, pumpMicros);
        if (observed === priorMax)
          break;
        priorMax = observed;
      }
      if (pumpResult < 0) {
        fail(`Rust audio pump failed: ${pumpResult}`);
        return;
      }
      if (pumpResult > 0) {
        if (!pcmWriter.tryWrite(pcmQuantum)) {
          fail("audio PCM publication failed");
          return;
        }
        continue;
      }
    }
    if (pcmWriter.depth >= pcmWriter.slotCapacity) {
      const now = performance.now();
      if (now >= nextSimulationMs) {
        simulationPhase += 0.025;
        const simulationStatus = wasm.afterglow_audio_simulate_motion(simulationPhase);
        if (simulationStatus !== 0) {
          fail(`Rust audio simulation failed: ${simulationStatus}`);
          return;
        }
        nextSimulationMs = now + 1000;
        continue;
      }
    }
    if (!serviced) {
      const epoch = Atomics.load(pcmTelemetry, 0 /* ConsumeEpoch */);
      if (pcmWriter.depth >= pcmWriter.slotCapacity)
        Atomics.wait(pcmTelemetry, 0 /* ConsumeEpoch */, epoch, 1);
      else if (attemptedPump && pumpResult === 0)
        Atomics.wait(pcmTelemetry, 0 /* ConsumeEpoch */, epoch, 1);
    }
  }
}
function fail(message) {
  if (pcmTelemetry !== null)
    Atomics.store(pcmTelemetry, 5 /* Fatal */, 1);
  self.postMessage({ type: "error", message });
}
