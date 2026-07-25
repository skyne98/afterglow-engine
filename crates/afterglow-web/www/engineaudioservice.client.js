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
export {
  EngineAudioServiceClient
};
