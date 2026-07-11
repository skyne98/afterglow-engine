// afterglow-web main-thread RPC client (shared by worker-test/worker-bench).
//
// Instantiates the shared main wasm + a Web Worker. `call(method, args)`
// writes `[method:u32][args]` to the request ring (the wasm `write_frame` auto-
// wakes the worker via the imported `notify_worker`), awaits the worker's
// response wake (postMessage carries NO payload — the ring is the transport),
// reads the response ring, and unwraps the postcard `Response`. One in-flight
// call at a time (SPSC). No busy-spinning: the main thread awaits a message.

const TIMEOUT_MS = 5000;

// --- minimal postcard (7-bit LEB128) codec helpers ------------------------

export function encodeVarint(n) {
  const b = [];
  do { let x = n & 0x7f; n = Math.floor(n / 128); if (n) x |= 0x80; b.push(x); } while (n);
  return b;
}
export function decodeVarint(bytes, off) {
  let r = 0;
  for (let shift = 0; shift < 35; shift += 7) {
    if (off >= bytes.length) throw new Error('postcard varint truncated');
    const b = bytes[off++];
    if (shift === 28 && (b & 0xf0)) throw new Error('postcard varint overflows u32');
    r += (b & 0x7f) * 2 ** shift;
    if (!(b & 0x80)) return [r >>> 0, off];
  }
  throw new Error('postcard varint overflows u32');
}
export function concat(...arrs) {
  const out = new Uint8Array(arrs.reduce((s, a) => s + a.length, 0));
  let o = 0;
  for (const a of arrs) { out.set(a, o); o += a.length; }
  return out;
}
// postcard(Vec<f32>) = [varint count][count x f32 LE]
export function encodeF32Vec(vec) {
  const v = encodeVarint(vec.length), out = new Uint8Array(v.length + vec.length * 4);
  out.set(v, 0);
  const dv = new DataView(out.buffer, out.byteOffset + v.length, vec.length * 4);
  for (let i = 0; i < vec.length; i++) dv.setFloat32(i * 4, vec[i], true);
  return out;
}
export function encodeF32(x) {
  const b = new Uint8Array(4);
  new DataView(b.buffer).setFloat32(0, x, true);
  return b;
}
export function decodeF32Vec(bytes) {
  const [n, off] = decodeVarint(bytes, 0);
  if (n > Math.floor((bytes.length - off) / 4)) throw new Error('postcard f32 vector truncated');
  const out = new Float32Array(n);
  const dv = new DataView(bytes.buffer, bytes.byteOffset + off, n * 4);
  for (let i = 0; i < n; i++) out[i] = dv.getFloat32(i * 4, true);
  return out;
}

// Unwrap a postcard `Response`: Ok(Vec<u8>) -> payload bytes, or throw a
// Server/Decode error. postcard encodes u32 and String lengths as LEB128:
//  Ok(payload)             = [0][varint len][bytes]
//  Server{method,message} = [1][varint method][varint len][bytes]
//  Decode{method,message} = [2][varint method][varint len][bytes]
export function unwrapResponse(bytes) {
  const [variant, off] = decodeVarint(bytes, 0);
  if (variant === 0) {
    const [plen, poff] = decodeVarint(bytes, off);
    if (poff + plen > bytes.length) throw new Error('RPC response truncated');
    return bytes.subarray(poff, poff + plen);
  }
  const [method, moff] = decodeVarint(bytes, off);
  const [mlen, eoff] = decodeVarint(bytes, moff);
  if (eoff + mlen > bytes.length) throw new Error('RPC error truncated');
  const msg = new TextDecoder().decode(bytes.subarray(eoff, eoff + mlen));
  throw new Error(`RPC ${variant === 1 ? 'server' : 'decode'} error (method ${method}): ${msg}`);
}

export class Rpc {
  /// Instantiate the shared main wasm + spawn the worker; resolve once the
  /// worker reports ready. `notify_worker` is wired to wake the worker.
  static async create({ mainWasmUrl, workerJsUrl, workerWasmUrl, timeoutMs }) {
    const memory = new WebAssembly.Memory({ shared: true, initial: 256, maximum: 1024 });
    const worker = new Worker(workerJsUrl, { type: 'module' });
    let rpc = null;
    try {
      const { exports: wasm } = await WebAssembly.instantiate(
        await WebAssembly.compile(await (await fetch(mainWasmUrl)).arrayBuffer()),
        { env: { memory, notify_worker: () => worker.postMessage('wake') } },
      );
      wasm.init_ring_buffers();
      rpc = new Rpc(wasm, memory, worker, { timeoutMs });
      worker.postMessage({
        type: 'init', sab: memory.buffer,
        reqBase: wasm.get_request_ptr(), respBase: wasm.get_response_ptr(),
        bufSize: wasm.get_buffer_size(), wasmUrl: workerWasmUrl,
      });
      await rpc._initPromise;
      worker.postMessage({ type: 'run' });
      return rpc;
    } catch (e) {
      // Setup or _initPromise failed: never leak a worker. terminate() latches
      // failure (idempotent if already poisoned) and stops the worker once.
      if (rpc) rpc.terminate(); else worker.terminate();
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
    this._fatal = null;        // latched transport failure (poison); null = healthy
    this._terminated = false;  // guard so worker.terminate() runs at most once
    this.timeoutMs = opts.timeoutMs ?? TIMEOUT_MS;
    this._initPromise = new Promise((res, rej) => { this._resolve = res; this._reject = rej; });
    this._initTimer = setTimeout(() => this._fail(new Error('worker init timeout')), this.timeoutMs);
    worker.onmessage = (e) => this._onmsg(e.data);
    worker.onerror = (e) => this._fail(new Error('worker: ' + ((e && e.message) || e)));
  }

  _onmsg(d) {
    // Poisoned: drop everything. A late response wake must never be read as a
    // later call's result, and a late ready/error must not resurrect state.
    if (this._fatal) return;
    if (d && d.type === 'ready') { clearTimeout(this._initTimer); const r = this._resolve; this._resolve = this._reject = null; if (r) r(); return; }
    if (d && d.type === 'error') { this._fail(this._reject ? new Error('worker init: ' + (d.message || 'error')) : new Error(d.message || 'worker error')); return; }
    if (this.pending) this._readResponse(); // response wake (no payload)
  }

  /// One in-flight SPSC call: write [method][args] to scratch, write_frame
  /// (auto-wakes the worker), then await the response wake.
  async call(method, args) {
    if (this._fatal) throw this._fatal; // poisoned: never touch the rings again
    if (this.pending) throw new Error('RPC busy: one in-flight call at a time');
    const len = 4 + args.length;
    if (len > this.scratchLen) throw new Error('request too large for scratch');
    const view = new Uint8Array(this.mem.buffer, this.scratch, len);
    view[0] = method & 0xff;
    view[1] = (method >>> 8) & 0xff;
    view[2] = (method >>> 16) & 0xff;
    view[3] = (method >>> 24) & 0xff;
    view.set(args, 4);
    if (this.w.write_frame(this.scratch, len) !== 0) throw new Error('write_frame failed (ring full)');
    return new Promise((resolve, reject) => {
      this.pending = { resolve, reject };
      this.pending.timer = setTimeout(() => this._fail(new Error('RPC timeout')), this.timeoutMs);
    });
  }

  _readResponse() {
    const n = this.w.read_response(this.scratch, this.scratchLen);
    const p = this.pending;
    this.pending = null;
    if (p) clearTimeout(p.timer);
    if (!p) return;
    if (n < 0) { p.reject(new Error('read_response returned ' + n)); return; }
    try { p.resolve(unwrapResponse(new Uint8Array(this.mem.buffer, this.scratch, n))); }
    catch (e) { p.reject(e); }
  }

  // Latch a fatal transport failure permanently. Idempotent: the first failure
  // wins; later _fail/terminate calls and late wakes are no-ops. Rejects the
  // init promise if still pending and/or the pending call if any. With no
  // pending work the error is still retained in `_fatal` so the next call fails
  // immediately.
  _fail(err) {
    if (this._fatal) return;
    this._fatal = err;
    clearTimeout(this._initTimer);
    if (this._reject) { const r = this._reject; this._resolve = this._reject = null; r(err); }
    if (this.pending) { const p = this.pending; this.pending = null; clearTimeout(p.timer); p.reject(err); }
  }

  terminate() {
    this._fail(new Error('terminated'));
    if (!this._terminated) { this._terminated = true; this.worker.terminate(); }
  }
}
