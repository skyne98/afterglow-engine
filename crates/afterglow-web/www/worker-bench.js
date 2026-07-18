// crates/afterglow-web/www/codec.ts
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
function encodeF32(x) {
  const b = new Uint8Array(4);
  new DataView(b.buffer).setFloat32(0, x, true);
  return b;
}
function decodeBool(bytes, off) {
  if (off >= bytes.length)
    throw new Error("postcard bool truncated");
  return [bytes[off] !== 0, off + 1];
}
function encodeF32Vec(vec) {
  const v = encodeVarint(vec.length);
  const out = new Uint8Array(v.length + vec.length * 4);
  out.set(v, 0);
  const dv = new DataView(out.buffer, out.byteOffset + v.length, vec.length * 4);
  for (let i = 0;i < vec.length; i++)
    dv.setFloat32(i * 4, vec[i], true);
  return out;
}
function decodeF32Vec(bytes, off) {
  const [n, o] = decodeVarint(bytes, off);
  const end = o + n * 4;
  if (end > bytes.length)
    throw new Error("postcard f32 vec truncated");
  const out = new Float32Array(n);
  const dv = new DataView(bytes.buffer, bytes.byteOffset + o, n * 4);
  for (let i = 0;i < n; i++)
    out[i] = dv.getFloat32(i * 4, true);
  return [out, end];
}

// crates/afterglow-web/www/rpc.ts
var TIMEOUT_MS = 5000;
function decodeVarint2(bytes, off) {
  let r = 0;
  for (let shift = 0;shift < 35; shift += 7) {
    if (off >= bytes.length)
      throw new Error("postcard varint truncated");
    const b = bytes[off++];
    if (shift === 28 && b & 240)
      throw new Error("postcard varint overflows u32");
    r += (b & 127) * 2 ** shift;
    if (!(b & 128))
      return [r >>> 0, off];
  }
  throw new Error("postcard varint overflows u32");
}
function unwrapResponse(bytes) {
  const [variant, off] = decodeVarint2(bytes, 0);
  if (variant === 0) {
    const [plen, poff] = decodeVarint2(bytes, off);
    if (poff + plen > bytes.length)
      throw new Error("RPC response truncated");
    return bytes.subarray(poff, poff + plen);
  }
  const [method, moff] = decodeVarint2(bytes, off);
  const [mlen, eoff] = decodeVarint2(bytes, moff);
  if (eoff + mlen > bytes.length)
    throw new Error("RPC error truncated");
  const msg = new TextDecoder().decode(bytes.subarray(eoff, eoff + mlen));
  throw new Error(`RPC ${variant === 1 ? "server" : "decode"} error (method ${method}): ${msg}`);
}

class Rpc {
  static async create({ mainWasmUrl, workerJsUrl, workerWasmUrl, timeoutMs }) {
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
        wasmUrl: workerWasmUrl
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

// crates/afterglow-web/www/physics.client.ts
class PhysicsClient {
  rpc;
  closed = false;
  static async spawn(opts = {}) {
    const rpc = await Rpc.create({
      mainWasmUrl: opts.mainWasmUrl ?? "afterglow_web.wasm",
      workerJsUrl: opts.workerJsUrl ?? "worker.js",
      workerWasmUrl: opts.workerWasmUrl ?? "physics.wasm",
      timeoutMs: opts.timeoutMs
    });
    return new PhysicsClient(rpc);
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
  async step(state, dt) {
    const args = concat(encodeF32Vec(state), encodeF32(dt));
    const resp = await this.rpc.call(0, args);
    return decodeF32Vec(resp, 0)[0];
  }
  async applyForce(bodyId, fx, fy, fz) {
    const args = concat(encodeU32(bodyId), encodeF32(fx), encodeF32(fy), encodeF32(fz));
    const resp = await this.rpc.call(1, args);
    return decodeBool(resp, 0)[0];
  }
}

// crates/afterglow-web/www/worker-bench.ts
var output = document.getElementById("out");
function print(message) {
  if (output)
    output.textContent += `${message}
`;
}
print("=== Typed Cross-Thread Worker Benchmark (Web Worker + SAB ring) ===");
print(`Service RPC includes typed postcard encode/decode and cross-thread notification.
`);
var client = await PhysicsClient.spawn({ workerWasmUrl: "physics_worker.wasm" });
try {
  const sizes = new Uint32Array([1, 4, 16, 64, 256, 1024, 4096, 16384]);
  const iterations = 1000, dt = 0.016, roundedDt = Math.fround(dt);
  const warmup = new Float32Array(64);
  for (let index = 0;index < warmup.length; index++)
    warmup[index] = index;
  for (let index = 0;index < 100; index++)
    await client.step(warmup, dt);
  print("  f32 count   payload   latency    bandwidth    valid/total");
  for (const count of sizes) {
    const input = new Float32Array(count);
    for (let index = 0;index < count; index++)
      input[index] = index;
    let elapsed = 0, valid = 0;
    for (let iteration = 0;iteration < iterations; iteration++) {
      const started = performance.now();
      const result = await client.step(input, dt);
      elapsed += performance.now() - started;
      if (result.length === count && result.every((value, index) => Math.abs(value - Math.fround((input[index] ?? 0) + roundedDt)) < 0.000001))
        valid++;
    }
    const latency = elapsed * 1000 / iterations;
    const bandwidth = count * 4 * iterations * 2 / (elapsed / 1000) / 1048576;
    print(`  ${String(count).padStart(9)}  ${String(count * 4).padStart(6)} B  ${latency.toFixed(1).padStart(7)} µs  ${bandwidth.toFixed(1).padStart(8)} MiB/s  ${valid}/${iterations} ${valid === iterations ? "OK" : "PARTIAL"}`);
  }
  print(`
Done.`);
} finally {
  client.close();
}
