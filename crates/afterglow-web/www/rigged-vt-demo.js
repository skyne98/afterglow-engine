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
function decodeU16(bytes, off) {
  return decodeVarint(bytes, off);
}
function encodeU32(n) {
  return new Uint8Array(encodeVarint(n));
}
function encodeF32(x) {
  const b = new Uint8Array(4);
  new DataView(b.buffer).setFloat32(0, x, true);
  return b;
}
function encodeBytes(b) {
  return concat(encodeVarint(b.length), b);
}
function decodeBytes(bytes, off) {
  const [len, o] = decodeVarint(bytes, off);
  const end = o + len;
  if (end > bytes.length)
    throw new Error("postcard bytes truncated");
  return [bytes.subarray(o, end), end];
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
function encodeU32Vec(vec) {
  const parts = [encodeVarint(vec.length)];
  for (let i = 0;i < vec.length; i++)
    parts.push(encodeVarint(vec[i]));
  return concat(...parts);
}
function decodeU32Vec(bytes, off) {
  const [n, o] = decodeVarint(bytes, off);
  const out = new Uint32Array(n);
  let pos = o;
  for (let i = 0;i < n; i++) {
    const [val, next] = decodeVarint(bytes, pos);
    out[i] = val;
    pos = next;
  }
  return [out, pos];
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

// crates/afterglow-web/www/async-worker.ts
class PendingFetch {
  constructor(url) {
    this.promise = fetch(url);
    this.resolved = false;
    this.bytes = null;
    this.error = null;
    this.promise.then(async (resp) => {
      if (!resp.ok) {
        this.error = new Error(`fetch ${resp.status}: ${url}`);
      } else {
        this.bytes = new Uint8Array(await resp.arrayBuffer());
      }
      this.resolved = true;
    }).catch((e) => {
      this.error = e;
      this.resolved = true;
    });
  }
}

class HeadFetch {
  constructor(url) {
    this.promise = fetch(url, { method: "HEAD" });
    this.resolved = false;
    this.contentLength = null;
    this.error = null;
    this.promise.then((resp) => {
      if (!resp.ok) {
        this.error = new Error(`HEAD ${resp.status}: ${url}`);
      } else {
        const cl = resp.headers.get("Content-Length");
        this.contentLength = cl ? parseInt(cl, 10) : null;
      }
      this.resolved = true;
    }).catch((e) => {
      this.error = e;
      this.resolved = true;
    });
  }
}

class RangeFetch {
  constructor(url, offset, len) {
    const start = Number(offset);
    const end = start + Number(len) - 1;
    this.promise = fetch(url, { headers: { Range: `bytes=${start}-${end}` } });
    this.resolved = false;
    this.bytes = null;
    this.error = null;
    this.promise.then(async (resp) => {
      if (!resp.ok && resp.status !== 206) {
        this.error = new Error(`range fetch ${resp.status}: ${url}`);
      } else {
        this.bytes = new Uint8Array(await resp.arrayBuffer());
      }
      this.resolved = true;
    }).catch((e) => {
      this.error = e;
      this.resolved = true;
    });
  }
}

class AsyncWorker {
  constructor(wasm, baseUrl = "") {
    this.w = wasm;
    this.baseUrl = baseUrl;
    this._memory = null;
    this.nextFetchId = 1;
    this._fetchCapacity = 256;
    this._fetchIds = new Float64Array(this._fetchCapacity);
    this._fetchIds.fill(-1);
    this._fetches = new Array(this._fetchCapacity).fill(null);
    this._pendingFetchCount = 0;
    this._callCapacity = 256;
    this._callIds = new Float64Array(this._callCapacity);
    this._callIds.fill(-1);
    this._callResolves = new Array(this._callCapacity).fill(null);
    this._callRejects = new Array(this._callCapacity).fill(null);
    this._pendingCallCount = 0;
    this._taskIdCounter = 0;
    this._pumpScheduled = false;
    this._completionLimit = 32;
    this._lastPollCompletions = 0;
    this._totalCompletions = 0;
    this._completionLimitHits = 0;
  }
  async call(method, args) {
    const taskId = this._nextTaskId();
    const slot = taskId % this._callCapacity;
    if (this._callIds[slot] !== -1)
      throw new Error("async worker: fixed task capacity exhausted");
    return new Promise((resolve, reject) => {
      this._callIds[slot] = taskId;
      this._callResolves[slot] = resolve;
      this._callRejects[slot] = reject;
      this._pendingCallCount++;
      if (this.serveAsync(method, args, taskId) < 0) {
        this._releaseCallSlot(slot);
        reject(new Error("async worker: serveAsync failed"));
        return;
      }
      this._schedulePump();
    });
  }
  _schedulePump() {
    if (this._pumpScheduled || this._pendingCallCount === 0)
      return;
    this._pumpScheduled = true;
    setTimeout(() => {
      this._pumpScheduled = false;
      if (this._pendingCallCount === 0)
        return;
      this.poll();
      this._schedulePump();
    }, 0);
  }
  serveAsync(method, args, taskId = this._nextTaskId()) {
    const inPtr = this.w.afterglow_wasm_input_ptr();
    const inSize = this.w.afterglow_wasm_input_size();
    if (args.length + 12 > inSize) {
      console.error("async worker: args too large for input scratch");
      return -1;
    }
    const view = new DataView((this._memory || this.w.memory).buffer, inPtr, 12 + args.length);
    view.setUint32(0, method, true);
    view.setBigUint64(4, BigInt(taskId), true);
    new Uint8Array((this._memory || this.w.memory).buffer, inPtr + 12, args.length).set(args);
    const r = this.w.afterglow_wasm_serve_async(method, inPtr + 12, args.length, BigInt(taskId));
    if (r < 0)
      return -1;
    return taskId;
  }
  poll(maxCompletions = this._completionLimit) {
    this.w.afterglow_wasm_tick();
    const outPtr = this.w.afterglow_wasm_output_ptr();
    const outSize = this.w.afterglow_wasm_output_size();
    const memory = this._memory || this.w.memory;
    let drained = 0;
    while (drained < maxCompletions) {
      const n = this.w.afterglow_wasm_drain_completion(outPtr, outSize);
      if (n < 0)
        break;
      if (n < 8)
        continue;
      const taskId = Number(new DataView(memory.buffer, outPtr, 8).getBigUint64(0, true));
      const responseBytes = new Uint8Array(memory.buffer, outPtr + 8, n - 8).slice();
      drained++;
      const slot = taskId % this._callCapacity;
      if (this._callIds[slot] === taskId) {
        const resolve = this._callResolves[slot];
        const reject = this._callRejects[slot];
        this._releaseCallSlot(slot);
        try {
          resolve(unwrapResponse(responseBytes));
        } catch (e) {
          reject(e);
        }
      }
    }
    this._lastPollCompletions = drained;
    this._totalCompletions += drained;
    if (drained === maxCompletions)
      this._completionLimitHits++;
    return drained;
  }
  fetchStart(urlPtr, urlLen) {
    const url = new TextDecoder().decode(Uint8Array.from(new Uint8Array((this._memory || this.w.memory).buffer, urlPtr, urlLen)));
    const fullUrl = this._resolveUrl(url);
    return this._registerFetch(new PendingFetch(fullUrl));
  }
  fetchPoll(fetchId, outPtr, outMax) {
    const pending = this._getFetch(fetchId);
    if (!pending)
      return -1;
    if (!pending.resolved)
      return -1;
    this._releaseFetch(fetchId);
    if (pending.error) {
      return 0;
    }
    if (pending.bytes.length > outMax)
      return -2;
    new Uint8Array((this._memory || this.w.memory).buffer, outPtr, outMax).set(pending.bytes);
    return pending.bytes.length;
  }
  headStart(urlPtr, urlLen) {
    const url = new TextDecoder().decode(Uint8Array.from(new Uint8Array((this._memory || this.w.memory).buffer, urlPtr, urlLen)));
    const fullUrl = this._resolveUrl(url);
    return this._registerFetch(new HeadFetch(fullUrl));
  }
  headPoll(fetchId, outPtr, outMax) {
    const pending = this._getFetch(fetchId);
    if (!pending)
      return -2;
    if (!pending.resolved)
      return -1;
    this._releaseFetch(fetchId);
    if (pending.error || pending.contentLength === null || outMax < 8)
      return -2;
    new DataView((this._memory || this.w.memory).buffer, outPtr, 8).setBigUint64(0, BigInt(pending.contentLength), true);
    return 8;
  }
  rangeStart(urlPtr, urlLen, offset, len) {
    const url = new TextDecoder().decode(Uint8Array.from(new Uint8Array((this._memory || this.w.memory).buffer, urlPtr, urlLen)));
    const fullUrl = this._resolveUrl(url);
    return this._registerFetch(new RangeFetch(fullUrl, offset, len));
  }
  _registerFetch(fetch2) {
    for (let probe = 0;probe < this._fetchCapacity; probe++) {
      const id = this.nextFetchId++;
      const slot = id % this._fetchCapacity;
      if (this._fetchIds[slot] !== -1)
        continue;
      this._fetchIds[slot] = id;
      this._fetches[slot] = fetch2;
      this._pendingFetchCount++;
      return id;
    }
    return 0;
  }
  _getFetch(id) {
    const slot = id % this._fetchCapacity;
    return this._fetchIds[slot] === id ? this._fetches[slot] : null;
  }
  _releaseFetch(id) {
    const slot = id % this._fetchCapacity;
    if (this._fetchIds[slot] !== id)
      return;
    this._fetchIds[slot] = -1;
    this._fetches[slot] = null;
    this._pendingFetchCount--;
  }
  _releaseCallSlot(slot) {
    this._callIds[slot] = -1;
    this._callResolves[slot] = null;
    this._callRejects[slot] = null;
    this._pendingCallCount--;
  }
  _nextTaskId() {
    return ++this._taskIdCounter;
  }
  _resolveUrl(path) {
    if (!this.baseUrl)
      return path;
    const p = path.startsWith("/") ? path.slice(1) : path;
    const sep = this.baseUrl.endsWith("/") ? "" : "/";
    return `${this.baseUrl}${sep}${p}`;
  }
}
function asyncWorkerImports(driver, memory) {
  driver._memory = memory;
  return {
    env: {
      memory,
      notify_worker: () => {},
      performance_now: () => performance.now(),
      ag_fetch_start: (urlPtr, urlLen) => driver.fetchStart(urlPtr, urlLen),
      ag_fetch_poll: (fetchId, outPtr, outMax) => driver.fetchPoll(fetchId, outPtr, outMax),
      ag_fetch_head_start: (urlPtr, urlLen) => driver.headStart(urlPtr, urlLen),
      ag_fetch_head_poll: (fetchId, outPtr, outMax) => driver.headPoll(fetchId, outPtr, outMax),
      ag_fetch_range_start: (urlPtr, urlLen, offset, len) => driver.rangeStart(urlPtr, urlLen, offset, len)
    }
  };
}

// crates/afterglow-web/www/meshopt.client.ts
class MeshoptClient {
  rpc;
  static async spawn(workerWasmUrl = "meshopt.wasm", baseUrl = "") {
    const driver = new AsyncWorker(null, baseUrl);
    const memory = new WebAssembly.Memory({ shared: true, initial: 256, maximum: 1024 });
    const { instance } = await WebAssembly.instantiate(await (await fetch(workerWasmUrl)).arrayBuffer(), asyncWorkerImports(driver, memory));
    driver.w = instance.exports;
    instance.exports.afterglow_wasm_init();
    return new MeshoptClient(driver);
  }
  poll() {
    this.rpc.poll();
  }
  constructor(rpc) {
    this.rpc = rpc;
  }
  async simplify(indices, positions, positionStride, targetIndexCount, targetError) {
    const args = concat(encodeU32Vec(indices), encodeF32Vec(positions), encodeU32(positionStride), encodeU32(targetIndexCount), encodeF32(targetError));
    const resp = await this.rpc.call(0, args);
    return decodeU32Vec(resp, 0)[0];
  }
  async simplifySloppy(indices, positions, positionStride, targetIndexCount, targetError) {
    const args = concat(encodeU32Vec(indices), encodeF32Vec(positions), encodeU32(positionStride), encodeU32(targetIndexCount), encodeF32(targetError));
    const resp = await this.rpc.call(1, args);
    return decodeU32Vec(resp, 0)[0];
  }
  async simplifyWithUvs(indices, positions, positionStride, uvs, uvStride, uvWeight, targetIndexCount, targetError) {
    const args = concat(encodeU32Vec(indices), encodeF32Vec(positions), encodeU32(positionStride), encodeF32Vec(uvs), encodeU32(uvStride), encodeF32(uvWeight), encodeU32(targetIndexCount), encodeF32(targetError));
    const resp = await this.rpc.call(2, args);
    return decodeU32Vec(resp, 0)[0];
  }
  async optimizeVertexCache(indices, vertexCount) {
    const args = concat(encodeU32Vec(indices), encodeU32(vertexCount));
    const resp = await this.rpc.call(3, args);
    return decodeU32Vec(resp, 0)[0];
  }
  async optimizeOverdraw(indices, positions, positionStride, threshold) {
    const args = concat(encodeU32Vec(indices), encodeF32Vec(positions), encodeU32(positionStride), encodeF32(threshold));
    const resp = await this.rpc.call(4, args);
    return decodeU32Vec(resp, 0)[0];
  }
  async encodeIndexBuffer(indices, vertexCount) {
    const args = concat(encodeU32Vec(indices), encodeU32(vertexCount));
    const resp = await this.rpc.call(5, args);
    return decodeBytes(resp, 0)[0];
  }
  async decodeIndexBuffer(buffer, indexCount) {
    const args = concat(encodeBytes(buffer), encodeU32(indexCount));
    const resp = await this.rpc.call(6, args);
    return decodeU32Vec(resp, 0)[0];
  }
  async encodeVertexBuffer(vertices, vertexSize) {
    const args = concat(encodeBytes(vertices), encodeU32(vertexSize));
    const resp = await this.rpc.call(7, args);
    return decodeBytes(resp, 0)[0];
  }
  async decodeVertexBuffer(buffer, vertexCount, vertexSize) {
    const args = concat(encodeBytes(buffer), encodeU32(vertexCount), encodeU32(vertexSize));
    const resp = await this.rpc.call(8, args);
    return decodeBytes(resp, 0)[0];
  }
  async generateVertexRemap(indices, vertices, vertexSize) {
    const args = concat(encodeU32Vec(indices), encodeBytes(vertices), encodeU32(vertexSize));
    const resp = await this.rpc.call(9, args);
    return decodeU32Vec(resp, 0)[0];
  }
  async stripify(indices, vertexCount, restartIndex) {
    const args = concat(encodeU32Vec(indices), encodeU32(vertexCount), encodeU32(restartIndex));
    const resp = await this.rpc.call(10, args);
    return decodeU32Vec(resp, 0)[0];
  }
  async buildMeshlets(indices, positions, positionStride, maxVertices, maxTriangles, coneWeight) {
    const args = concat(encodeU32Vec(indices), encodeF32Vec(positions), encodeU32(positionStride), encodeU32(maxVertices), encodeU32(maxTriangles), encodeF32(coneWeight));
    const resp = await this.rpc.call(11, args);
    return decodeBytes(resp, 0)[0];
  }
  async analyzeVertexCache(indices, vertexCount) {
    const args = concat(encodeU32Vec(indices), encodeU32(vertexCount));
    const resp = await this.rpc.call(12, args);
    return decodeF32Vec(resp, 0)[0];
  }
  async quantizeHalf(value) {
    const args = encodeF32(value);
    const resp = await this.rpc.call(13, args);
    return decodeU16(resp, 0)[0];
  }
}

// crates/afterglow-web/www/texture.client.ts
class TextureClient {
  rpc;
  static async spawn(workerWasmUrl = "texture.wasm", baseUrl = "") {
    const driver = new AsyncWorker(null, baseUrl);
    const memory = new WebAssembly.Memory({ shared: true, initial: 256, maximum: 1024 });
    const { instance } = await WebAssembly.instantiate(await (await fetch(workerWasmUrl)).arrayBuffer(), asyncWorkerImports(driver, memory));
    driver.w = instance.exports;
    instance.exports.afterglow_wasm_init();
    return new TextureClient(driver);
  }
  poll() {
    this.rpc.poll();
  }
  constructor(rpc) {
    this.rpc = rpc;
  }
  async transcode(data, targetFormat) {
    const args = concat(encodeBytes(data), encodeU32(targetFormat));
    const resp = await this.rpc.call(0, args);
    return decodeBytes(resp, 0)[0];
  }
  async generateMips(data, width, height) {
    const args = concat(encodeBytes(data), encodeU32(width), encodeU32(height));
    const resp = await this.rpc.call(1, args);
    return decodeBytes(resp, 0)[0];
  }
  async downscale(data, width, height, targetWidth, targetHeight) {
    const args = concat(encodeBytes(data), encodeU32(width), encodeU32(height), encodeU32(targetWidth), encodeU32(targetHeight));
    const resp = await this.rpc.call(2, args);
    return decodeBytes(resp, 0)[0];
  }
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
function unwrapResponse2(bytes) {
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
      p.resolve(unwrapResponse2(new Uint8Array(this.mem.buffer, this.scratch, n)));
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

// crates/afterglow-web/www/engine/big-parser.ts
function decodeVarint3(bytes, off) {
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
function decodeU32(bytes, off) {
  return decodeVarint3(bytes, off);
}
function decodeU64(bytes, off) {
  let result = 0n;
  for (let shift = 0n;shift < 70n; shift += 7n) {
    if (off >= bytes.length)
      throw new Error("postcard u64 varint truncated");
    const byte = bytes[off++];
    result |= BigInt(byte & 127) << shift;
    if (!(byte & 128)) {
      if (result > 0xffff_ffff_ffff_ffffn)
        throw new Error("postcard u64 varint overflows");
      return [result, off];
    }
  }
  throw new Error("postcard u64 varint overflows");
}
function decodeString(bytes, off) {
  const [len, o] = decodeVarint3(bytes, off);
  const str = new TextDecoder().decode(bytes.subarray(o, o + len));
  return [str, o + len];
}
function decodeVec(bytes, off, decodeFn) {
  const [len, o] = decodeVarint3(bytes, off);
  const result = [];
  let pos = o;
  for (let i = 0;i < len; i++) {
    const [item, newOff] = decodeFn(bytes, pos);
    result.push(item);
    pos = newOff;
  }
  return [result, pos];
}
function decodeBool(bytes, off) {
  return [bytes[off] !== 0, off + 1];
}
function decodeU8(bytes, off) {
  return [bytes[off], off + 1];
}
function decodeAssetType(bytes, off) {
  const [variant, o] = decodeU32(bytes, off);
  switch (variant) {
    case 0:
      return ["Texture", o];
    case 1:
      return ["Mesh", o];
    case 2:
      return ["VirtualTexture", o];
    default:
      throw new Error(`unknown AssetType variant: ${variant}`);
  }
}
function decodeCompression(bytes, off) {
  const [variant, o] = decodeU32(bytes, off);
  switch (variant) {
    case 0:
      return ["Meshopt", o];
    case 1:
      return ["None", o];
    default:
      throw new Error(`unknown Compression variant: ${variant}`);
  }
}
function decodeTextureEncoding(bytes, off) {
  const [variant, next] = decodeU32(bytes, off);
  if (variant === 0)
    return ["RawRgba8", next];
  if (variant === 1)
    return ["Basis", next];
  throw new Error(`unknown TextureEncoding variant: ${variant}`);
}
function decodeChunkMeta(bytes, off) {
  const [variant, o] = decodeU32(bytes, off);
  switch (variant) {
    case 0: {
      const [w, o2] = decodeU32(bytes, o);
      const [h, o3] = decodeU32(bytes, o2);
      return [{ type: "Texture", width: w, height: h }, o3];
    }
    case 1: {
      const [ic, o2] = decodeU32(bytes, o);
      const [vc, o3] = decodeU32(bytes, o2);
      const [ps, o4] = decodeU32(bytes, o3);
      const [us, o5] = decodeU32(bytes, o4);
      return [{ type: "Mesh", indexCount: ic, vertexCount: vc, positionStride: ps, uvStride: us }, o5];
    }
    case 2:
      return [{ type: "Raw" }, o];
    default:
      throw new Error(`unknown ChunkMeta variant: ${variant}`);
  }
}
function decodeChunkInfo(bytes, off) {
  const [offset, o1] = decodeU64(bytes, off);
  const [compressedSize, o2] = decodeU64(bytes, o1);
  const [uncompressedSize, o3] = decodeU64(bytes, o2);
  const [lodLevel, o4] = decodeU8(bytes, o3);
  const [mipLevel, o5] = decodeU8(bytes, o4);
  const [compression, o6] = decodeCompression(bytes, o5);
  const [meta, o7] = decodeChunkMeta(bytes, o6);
  return [{
    offset,
    compressedSize,
    uncompressedSize,
    lodLevel,
    mipLevel,
    compression,
    meta
  }, o7];
}
function decodeVTMipDirectory(bytes, off) {
  const [mip, o1] = decodeU8(bytes, off);
  const [pagesX, o2] = decodeU32(bytes, o1);
  const [pagesY, o3] = decodeU32(bytes, o2);
  const [offset, o4] = decodeU64(bytes, o3);
  const [pageSizes, o5] = decodeVec(bytes, o4, decodeU32);
  return [{ mip, pagesX, pagesY, offset, pageSizes }, o5];
}
function decodeVTTailDirectory(bytes, off) {
  const [firstMip, o1] = decodeU8(bytes, off);
  const [offset, o2] = decodeU64(bytes, o1);
  const [size, o3] = decodeU32(bytes, o2);
  return [{ firstMip, offset, size }, o3];
}
function decodeVTDirectory(bytes, off) {
  const [width, o1] = decodeU32(bytes, off);
  const [height, o2] = decodeU32(bytes, o1);
  const [encoding, o3] = decodeTextureEncoding(bytes, o2);
  const [mips, o4] = decodeVec(bytes, o3, decodeVTMipDirectory);
  const [hasTail, o5] = decodeBool(bytes, o4);
  if (!hasTail)
    return [{ width, height, encoding, mips, tail: null }, o5];
  const [tail, o6] = decodeVTTailDirectory(bytes, o5);
  return [{ width, height, encoding, mips, tail }, o6];
}
function decodeAssetEntry(bytes, off) {
  const [name, o1] = decodeString(bytes, off);
  const [assetType, o2] = decodeAssetType(bytes, o1);
  const [chunks, o3] = decodeVec(bytes, o2, decodeChunkInfo);
  const [hasVirtualTexture, o4] = decodeBool(bytes, o3);
  if (!hasVirtualTexture)
    return [{ name, assetType, chunks, virtualTexture: null }, o4];
  const [virtualTexture, o5] = decodeVTDirectory(bytes, o4);
  return [{ name, assetType, chunks, virtualTexture }, o5];
}
var BIG_MAGIC = 826755394;
var BIG_VERSION = 5;
function parseBigHeader(data) {
  if (data.length < 16)
    throw new Error(".big: file too small");
  const magic = new DataView(data.buffer, data.byteOffset, 4).getUint32(0, true);
  if (magic !== BIG_MAGIC)
    throw new Error(".big: bad magic");
  const version = new DataView(data.buffer, data.byteOffset + 4, 4).getUint32(0, true);
  if (version !== BIG_VERSION)
    throw new Error(`.big: version ${version} != ${BIG_VERSION}`);
  const dataOffset = Number(new DataView(data.buffer, data.byteOffset + 8, 8).getBigUint64(0, true));
  const headerBytes = data.subarray(16, dataOffset);
  let off = 0;
  const [hdrVersion, o1] = decodeU32(headerBytes, off);
  off = o1;
  const [hdrDataOffset, o2] = decodeU64(headerBytes, off);
  off = o2;
  const [assets, o3] = decodeVec(headerBytes, off, decodeAssetEntry);
  off = o3;
  return {
    header: { version: hdrVersion, dataOffset: hdrDataOffset, assets },
    dataOffset
  };
}
function getVirtualTextureDimensions(header, assetName) {
  const directory = header.assets.find((asset) => asset.name === assetName)?.virtualTexture;
  if (!directory)
    throw new Error(`VT dimensions unavailable: ${assetName}`);
  return { width: directory.width, height: directory.height };
}
class BoundedTranscoderPool {
  workers;
  jobs;
  workerBusy;
  head = 0;
  tail = 0;
  count = 0;
  active = 0;
  completed = 0;
  totalQueueMs = 0;
  maxQueueMs = 0;
  totalTranscodeMs = 0;
  maxTranscodeMs = 0;
  stats = {
    workerCount: 0,
    active: 0,
    queued: 0,
    completed: 0,
    averageQueueMs: 0,
    maxQueueMs: 0,
    averageTranscodeMs: 0,
    maxTranscodeMs: 0
  };
  constructor(workers, capacity) {
    this.workers = workers;
    if (workers.length === 0 || !Number.isInteger(capacity) || capacity < 1)
      throw new RangeError("VT transcoder pool requires workers and positive capacity");
    this.jobs = new Array(capacity).fill(null);
    this.workerBusy = new Uint8Array(workers.length);
  }
  submit(data, format, signal) {
    if (this.count === this.jobs.length)
      return Promise.reject(new Error("VT transcode queue capacity exceeded"));
    return new Promise((resolve, reject) => {
      this.jobs[this.tail] = { data, format, signal, queuedAt: performance.now(), resolve, reject };
      this.tail = (this.tail + 1) % this.jobs.length;
      this.count++;
      this.pump();
    });
  }
  pump() {
    for (let workerIndex = 0;workerIndex < this.workers.length && this.count !== 0; workerIndex++) {
      if (this.workerBusy[workerIndex] !== 0)
        continue;
      const job = this.jobs[this.head];
      this.jobs[this.head] = null;
      this.head = (this.head + 1) % this.jobs.length;
      this.count--;
      if (job.signal?.aborted) {
        job.reject(new Error("VT transcode canceled before dispatch"));
        workerIndex--;
        continue;
      }
      const queueMs = performance.now() - job.queuedAt;
      this.totalQueueMs += queueMs;
      this.maxQueueMs = Math.max(this.maxQueueMs, queueMs);
      this.workerBusy[workerIndex] = 1;
      this.active++;
      this.run(workerIndex, job);
    }
  }
  async run(workerIndex, job) {
    const startedAt = performance.now();
    try {
      const result = await this.workers[workerIndex].transcode(job.data, job.format);
      if (job.signal?.aborted)
        job.reject(new Error("VT transcode canceled after dispatch"));
      else
        job.resolve(result.slice());
    } catch (error) {
      job.reject(error);
    } finally {
      const elapsed = performance.now() - startedAt;
      this.completed++;
      this.totalTranscodeMs += elapsed;
      this.maxTranscodeMs = Math.max(this.maxTranscodeMs, elapsed);
      this.workerBusy[workerIndex] = 0;
      this.active--;
      this.pump();
    }
  }
  getStats() {
    const stats = this.stats;
    stats.workerCount = this.workers.length;
    stats.active = this.active;
    stats.queued = this.count;
    stats.completed = this.completed;
    stats.averageQueueMs = this.completed === 0 ? 0 : this.totalQueueMs / this.completed;
    stats.maxQueueMs = this.maxQueueMs;
    stats.averageTranscodeMs = this.completed === 0 ? 0 : this.totalTranscodeMs / this.completed;
    stats.maxTranscodeMs = this.maxTranscodeMs;
    return stats;
  }
}
function createFetchRangeLoader(baseUrl = "") {
  const url = (path) => baseUrl + path;
  const identity = async (path) => {
    const response = await fetch(url(path), { headers: { Range: "bytes=0-0" } });
    if (response.status !== 206)
      throw new Error(`asset identity range expected 206, got ${response.status}: ${path}`);
    const contentRange = response.headers.get("content-range") ?? "";
    const separator = contentRange.lastIndexOf("/");
    const size = Number(separator < 0 ? "" : contentRange.slice(separator + 1));
    if (!Number.isSafeInteger(size) || size < 1)
      throw new Error(`asset identity has invalid content-range: ${path}`);
    return {
      size,
      etag: response.headers.get("etag"),
      lastModified: response.headers.get("last-modified")
    };
  };
  return {
    async load(path) {
      const response = await fetch(url(path));
      if (!response.ok)
        throw new Error(`asset fetch ${response.status}: ${path}`);
      return new Uint8Array(await response.arrayBuffer());
    },
    async size(path) {
      return (await identity(path)).size;
    },
    identity,
    async read(path, offset, len) {
      if (!Number.isSafeInteger(offset) || offset < 0 || !Number.isSafeInteger(len) || len < 0)
        throw new RangeError("asset range must use non-negative safe integers");
      if (len === 0)
        return new Uint8Array(0);
      const response = await fetch(url(path), {
        headers: { Range: `bytes=${offset}-${offset + len - 1}` }
      });
      if (response.status !== 206)
        throw new Error(`asset range fetch expected 206, got ${response.status}: ${path}`);
      const bytes = new Uint8Array(await response.arrayBuffer());
      if (bytes.byteLength !== len)
        throw new Error(`asset range returned ${bytes.byteLength} bytes; expected ${len}: ${path}`);
      return bytes;
    }
  };
}

class BigContainerAssetLoader {
  source;
  containerPath;
  assets = new Map;
  constructor(source, containerPath, header) {
    this.source = source;
    this.containerPath = containerPath;
    for (const asset of header.assets) {
      if (asset.chunks.length !== 1 || asset.chunks[0].meta.type !== "Raw")
        continue;
      const chunk = asset.chunks[0];
      if (chunk.compression !== "None" || chunk.compressedSize !== chunk.uncompressedSize)
        throw new Error(`raw BIG asset must be uncompressed: ${asset.name}`);
      if (chunk.uncompressedSize > BigInt(Number.MAX_SAFE_INTEGER))
        throw new RangeError(`raw BIG asset exceeds browser safe size: ${asset.name}`);
      this.assets.set(asset.name, chunk);
    }
  }
  chunk(path) {
    const chunk = this.assets.get(path);
    if (!chunk)
      throw new Error(`raw BIG asset not found: ${path}`);
    return chunk;
  }
  load(path) {
    const chunk = this.chunk(path);
    return this.source.read(this.containerPath, Number(chunk.offset), Number(chunk.uncompressedSize));
  }
  async size(path) {
    return Number(this.chunk(path).uncompressedSize);
  }
  read(path, offset, length) {
    const chunk = this.chunk(path);
    const size = Number(chunk.uncompressedSize);
    if (!Number.isSafeInteger(offset) || offset < 0 || !Number.isSafeInteger(length) || length < 0 || offset + length > size)
      throw new RangeError(`raw BIG asset range exceeds ${path}: ${offset}+${length} > ${size}`);
    return this.source.read(this.containerPath, Number(chunk.offset) + offset, length);
  }
  poll() {}
}
function createPageDataProvider(loader, header, textureWorkers, format, cache) {
  const directories = new Map;
  for (let assetId = 0;assetId < header.assets.length; assetId++) {
    const asset = header.assets[assetId];
    const source = asset.virtualTexture;
    if (!source)
      continue;
    let maxMip = 0;
    for (const mip of source.mips)
      maxMip = Math.max(maxMip, mip.mip);
    const mips = new Array(maxMip + 1).fill(null);
    for (const mip of source.mips) {
      const sizes = Uint32Array.from(mip.pageSizes);
      const offsets = new Float64Array(sizes.length);
      let offset = Number(mip.offset);
      for (let page = 0;page < sizes.length; page++) {
        offsets[page] = offset;
        offset += sizes[page];
      }
      mips[mip.mip] = { pagesX: mip.pagesX, pagesY: mip.pagesY, offsets, sizes };
    }
    directories.set(asset.name, {
      assetId,
      encoding: source.encoding,
      mips,
      tailOffset: source.tail ? Number(source.tail.offset) : 0,
      tailSize: source.tail?.size ?? 0
    });
  }
  const transcoder = new BoundedTranscoderPool(textureWorkers, 64);
  let reads = 0;
  let totalReadMs = 0;
  let maxReadMs = 0;
  const stats = {
    reads: 0,
    averageReadMs: 0,
    maxReadMs: 0,
    workerCount: textureWorkers.length,
    activeTranscodes: 0,
    queuedTranscodes: 0,
    completedTranscodes: 0,
    averageTranscodeQueueMs: 0,
    maxTranscodeQueueMs: 0,
    averageTranscodeMs: 0,
    maxTranscodeMs: 0,
    cacheEnabled: cache !== undefined,
    cacheBackend: "",
    cacheEntries: 0,
    cacheBytes: 0,
    cacheLiveBytes: 0,
    cacheQueuedWrites: 0,
    cacheEvictions: 0,
    cacheCompactions: 0,
    cacheReclaimedBytes: 0,
    cacheMaintenance: false,
    cacheHits: 0,
    cacheMisses: 0,
    cacheWrites: 0,
    cacheRejected: 0,
    cacheErrors: 0,
    averageCacheReadMs: 0,
    maxCacheReadMs: 0,
    averageCacheWriteMs: 0,
    maxCacheWriteMs: 0
  };
  const provider = async (path, req, signal) => {
    if (signal?.aborted)
      throw new Error("VT page load canceled before read");
    const directory = directories.get(path);
    let offset = 0;
    let size = 0;
    if (req.tail) {
      offset = directory?.tailOffset ?? 0;
      size = directory?.tailSize ?? 0;
    } else {
      const mip = directory?.mips[req.mip];
      if (mip && req.x >= 0 && req.y >= 0 && req.x < mip.pagesX && req.y < mip.pagesY) {
        const page = req.y * mip.pagesX + req.x;
        offset = mip.offsets[page];
        size = mip.sizes[page];
      }
    }
    if (!directory || size === 0)
      throw new Error(`VT page not found: ${path} mip=${req.mip} (${req.x},${req.y})`);
    const cacheKey = `${directory.assetId}:${req.tail ? "t" : req.mip}:${req.x}:${req.y}`;
    const expectedBytes = format === 4 ? 136 * 136 * 4 : 34 * 34 * 16;
    if (cache) {
      const cached = await cache.get(cacheKey);
      if (signal?.aborted)
        throw new Error("VT page load canceled after cache read");
      if (cached && cached.byteLength === expectedBytes)
        return cached;
    }
    const readStartedAt = performance.now();
    const pageData = await loader.read(path + ".big", offset, size);
    const readMs = performance.now() - readStartedAt;
    reads++;
    totalReadMs += readMs;
    maxReadMs = Math.max(maxReadMs, readMs);
    if (signal?.aborted)
      throw new Error("VT page load canceled after read");
    if (directory.encoding === "RawRgba8") {
      if (format !== 4) {
        throw new Error(`VT page ${path} is raw RGBA8 but GPU format ${format} requires Basis encoding`);
      }
      if (cache)
        cache.put(cacheKey, pageData);
      return pageData;
    }
    if (pageData.byteLength < 2 || pageData[0] !== 115 || pageData[1] !== 66)
      throw new Error(`invalid Basis page range for ${path}: bytes=${pageData.byteLength}, magic=${pageData[0]},${pageData[1]}`);
    const transcoded = await transcoder.submit(pageData, format, signal);
    if (signal?.aborted)
      throw new Error("VT page load canceled after transcode");
    if (transcoded.byteLength < 16)
      throw new Error("truncated transcoded VT page");
    const view = new DataView(transcoded.buffer, transcoded.byteOffset, transcoded.byteLength);
    const count = view.getUint32(0, true);
    const width = view.getUint32(4, true);
    const height = view.getUint32(8, true);
    const length = view.getUint32(12, true);
    if (count < 1 || width !== 136 || height !== 136 || 16 + length > transcoded.byteLength)
      throw new Error(`invalid transcoded VT page header: count=${count}, size=${width}x${height}, bytes=${length}`);
    const payload = transcoded.slice(16, 16 + length);
    if (cache)
      cache.put(cacheKey, payload);
    return payload;
  };
  provider.getStats = () => {
    const transcode = transcoder.getStats();
    stats.reads = reads;
    stats.averageReadMs = reads === 0 ? 0 : totalReadMs / reads;
    stats.maxReadMs = maxReadMs;
    stats.workerCount = transcode.workerCount;
    stats.activeTranscodes = transcode.active;
    stats.queuedTranscodes = transcode.queued;
    stats.completedTranscodes = transcode.completed;
    stats.averageTranscodeQueueMs = transcode.averageQueueMs;
    stats.maxTranscodeQueueMs = transcode.maxQueueMs;
    stats.averageTranscodeMs = transcode.averageTranscodeMs;
    stats.maxTranscodeMs = transcode.maxTranscodeMs;
    const persistent = cache?.getStats();
    if (persistent) {
      stats.cacheBackend = persistent.backend;
      stats.cacheEntries = persistent.entries;
      stats.cacheBytes = persistent.bytes;
      stats.cacheLiveBytes = persistent.liveBytes;
      stats.cacheQueuedWrites = persistent.queuedWrites;
      stats.cacheEvictions = persistent.evictions;
      stats.cacheCompactions = persistent.compactions;
      stats.cacheReclaimedBytes = persistent.reclaimedBytes;
      stats.cacheMaintenance = persistent.maintenance;
      stats.cacheHits = persistent.hits;
      stats.cacheMisses = persistent.misses;
      stats.cacheWrites = persistent.writes;
      stats.cacheRejected = persistent.rejectedCapacity + persistent.rejectedQueue;
      stats.cacheErrors = persistent.corruptEntries + persistent.readErrors + persistent.writeErrors;
      stats.averageCacheReadMs = persistent.averageReadMs;
      stats.maxCacheReadMs = persistent.maxReadMs;
      stats.averageCacheWriteMs = persistent.averageWriteMs;
      stats.maxCacheWriteMs = persistent.maxWriteMs;
    }
    return stats;
  };
  return provider;
}

// crates/afterglow-web/www/engine/webgpu-only.ts
function disableWebGLFallback(renderer) {
  renderer._getFallback = null;
}
function assertWebGPUBackend(renderer) {
  if (renderer.backend?.isWebGPUBackend !== true || renderer.backend.device == null) {
    throw new Error("Afterglow requires a live WebGPU backend; WebGL fallback is forbidden.");
  }
}
function showWebGPUFailure(error) {
  const message = error instanceof Error ? error.message : String(error);
  const panel = document.createElement("pre");
  panel.id = "afterglow-webgpu-failure";
  panel.textContent = `Afterglow requires hardware WebGPU.

${message}`;
  panel.style.cssText = "box-sizing:border-box;margin:0;min-height:100vh;padding:24px;background:#11151c;color:#ff9a9a;font:16px/1.5 ui-monospace,monospace;white-space:pre-wrap";
  document.body.replaceChildren(panel);
  console.error("Afterglow WebGPU startup failed:", error);
}
var legacyWindowRendererFactory = (parameters) => {
  const legacyWindow = window;
  return new legacyWindow.THREE.WebGPURenderer(parameters);
};
async function createWebGPUOnlyRenderer(parameters = {}, factory) {
  const gpu = navigator.gpu;
  if (!gpu)
    throw new Error("navigator.gpu is unavailable. WebGL fallback is disabled.");
  const adapter = await gpu.requestAdapter();
  if (!adapter)
    throw new Error("Unable to acquire a hardware WebGPU adapter. WebGL fallback is disabled.");
  const renderer = factory(parameters);
  renderer.afterglowAdapterInfo = adapter.info;
  disableWebGLFallback(renderer);
  try {
    await renderer.init();
    assertWebGPUBackend(renderer);
  } catch (error) {
    renderer.dispose();
    throw error;
  }
  const onDeviceLost = renderer.onDeviceLost.bind(renderer);
  renderer.onDeviceLost = (info) => {
    onDeviceLost(info);
    showWebGPUFailure(new Error(`WebGPU device lost (${info.reason ?? "unknown"}): ${info.message ?? "no detail"}`));
  };
  return renderer;
}

// crates/afterglow-web/www/engine/renderer-seal.ts
class RendererSeal {
  backend;
  sealed = false;
  renderPipelines = 0;
  computePipelines = 0;
  renderPipelineViolations = 0;
  computePipelineViolations = 0;
  constructor(backend) {
    this.backend = backend;
    const originalRender = backend.createRenderPipeline.bind(backend);
    const originalCompute = backend.createComputePipeline.bind(backend);
    const monitor = this;
    backend.createRenderPipeline = function(renderObject, promises) {
      monitor.renderPipelines++;
      if (monitor.sealed)
        monitor.renderPipelineViolations++;
      originalRender(renderObject, promises);
    };
    backend.createComputePipeline = function(computePipeline, bindings) {
      monitor.computePipelines++;
      if (monitor.sealed)
        monitor.computePipelineViolations++;
      originalCompute(computePipeline, bindings);
    };
  }
  seal() {
    this.sealed = true;
  }
  get isSealed() {
    return this.sealed;
  }
  get violations() {
    return this.renderPipelineViolations + this.computePipelineViolations;
  }
  assertNoViolations() {
    if (this.violations !== 0)
      throw new Error(`renderer created ${this.violations} pipeline(s) after seal`);
  }
}
async function warmRendererVariants(renderer, variants) {
  for (const variant of variants)
    await renderer.compileAsync(variant.scene, variant.camera);
}

// crates/afterglow-web/www/rigged-vt-demo.ts
var THREE = window.THREE;
var VT = window.AfterglowVT;
var GLTFLoader = window.AfterglowLoaders?.GLTFLoader;
var AssetStore = window.AfterglowAssets?.AssetStore;
if (!VT || !GLTFLoader || !AssetStore)
  throw new Error("Afterglow VT/asset/loader bundle is unavailable");
var FEEDBACK_INTERVAL = 8;
var MODEL_LAYER = 1;
var errors = [];
var frame = 0;
var last = performance.now();
var smoothedDt = 1 / 60;
var lastResult = { totalRequests: 0 };
var feedbackResults = [null, null, null, null];
var mergedFeedback = new Map;
var animationEnabled = true;
var feedbackEnabled = true;
var programmatic = false;
var orbitAngle = 0;
var orbitVelocity = 0;
var cameraDistance = 4.1;
var zoomVelocity = 0;
var keys = new Set;
var waiters = [];
var scene = new THREE.Scene;
scene.background = new THREE.Color(1119258);
var camera = new THREE.PerspectiveCamera(48, innerWidth / innerHeight, 0.05, 100);
camera.position.set(0, 1.45, 4.1);
camera.lookAt(0, 1.25, 0);
camera.layers.enable(MODEL_LAYER);
var renderer = await createWebGPUOnlyRenderer({ antialias: false, trackTimestamp: false }, legacyWindowRendererFactory).catch((error) => {
  showWebGPUFailure(error);
  throw error;
});
renderer.setPixelRatio(devicePixelRatio);
renderer.setSize(innerWidth, innerHeight);
renderer.shadowMap.enabled = true;
renderer.shadowMap.type = THREE.PCFShadowMap;
document.body.append(renderer.domElement);
var rendererSeal = new RendererSeal(renderer.backend);
renderer.backend.device.addEventListener("uncapturederror", (event) => errors.push(String(event.error?.message ?? event.error)));
addEventListener("error", (event) => errors.push(String(event.error?.stack ?? event.message)));
addEventListener("unhandledrejection", (event) => errors.push(String(event.reason?.stack ?? event.reason)));
scene.add(new THREE.HemisphereLight(13162751, 2301210, 2.1));
var key = new THREE.DirectionalLight(16771792, 3.5);
key.position.set(3, 5, 4);
key.layers.enable(MODEL_LAYER);
key.castShadow = true;
key.shadow.mapSize.set(2048, 2048);
key.shadow.camera.left = -3.5;
key.shadow.camera.right = 3.5;
key.shadow.camera.top = 3.5;
key.shadow.camera.bottom = -3.5;
key.shadow.camera.near = 0.1;
key.shadow.camera.far = 12;
key.shadow.bias = -0.0004;
key.shadow.normalBias = 0.025;
scene.add(key);
var rim = new THREE.DirectionalLight(7838207, 2.2);
rim.position.set(-4, 3, -3);
rim.layers.enable(MODEL_LAYER);
scene.add(rim);
var floor = new THREE.Mesh(new THREE.CircleGeometry(3.2, 64), new THREE.MeshStandardMaterial({ color: 2369584, roughness: 0.9, metalness: 0 }));
floor.rotation.x = -Math.PI / 2;
floor.receiveShadow = true;
scene.add(floor);
var grid = new THREE.GridHelper(6, 12, 5464178, 3159616);
grid.position.y = 0.002;
scene.add(grid);
var rangeLoader = createFetchRangeLoader();
var workerCount = Math.max(2, Math.min(4, Math.floor((navigator.hardwareConcurrency || 4) / 2)));
var textureRpcs = await Promise.all(Array.from({ length: workerCount }, () => Rpc.create({
  mainWasmUrl: "afterglow_web.wasm",
  workerJsUrl: "worker.js",
  workerWasmUrl: "texture.wasm",
  timeoutMs: 1e4
})));
var textureWorkers = textureRpcs.map((rpc) => new TextureClient(rpc));
var meshopt = await MeshoptClient.spawn("meshopt.wasm");
addEventListener("beforeunload", () => {
  for (const rpc of textureRpcs)
    rpc.terminate();
}, { once: true });
var prefix = await rangeLoader.read("rigged-vt.big", 0, 16);
var dataOffset = Number(new DataView(prefix.buffer, prefix.byteOffset + 8, 8).getBigUint64(0, true));
var headerBytes = await rangeLoader.read("rigged-vt.big", 0, dataOffset);
var { header } = parseBigHeader(headerBytes);
var format = renderer.backend.device.features.has("texture-compression-bc") ? 0 : renderer.backend.device.features.has("texture-compression-astc") ? 1 : VT.FORMAT_RGBA;
var containerLoader = {
  load: (path) => rangeLoader.load(path),
  size: (path) => rangeLoader.size(path),
  read: (_path, offset, length) => rangeLoader.read("rigged-vt.big", offset, length)
};
var pageProvider = createPageDataProvider(containerLoader, header, textureWorkers, format);
var packedAssets = new BigContainerAssetLoader(rangeLoader, "rigged-vt.big", header);
var assetStore = new AssetStore(packedAssets, meshopt);
var modelHandle = assetStore.loadOptimizedGLTF("model.glb", new GLTFLoader);
var secondModelHandle = assetStore.loadOptimizedGLTF("model-2.glb", new GLTFLoader);
while (modelHandle.state === "loading" || secondModelHandle.state === "loading") {
  assetStore.poll();
  await new Promise((resolve) => requestAnimationFrame(() => resolve()));
}
if (modelHandle.state !== "ready" || secondModelHandle.state !== "ready")
  throw new Error("packed models failed GLTF parsing or runtime mesh optimization");
var store = new VT.VirtualTextureStore({ read: (path, offset, length) => rangeLoader.read(path, offset, length), poll() {} }, pageProvider, format, renderer.backend.device, new VT.VirtualTextureTuning);
var paths = {
  albedo: "model.glb#image-0",
  normal: "model.glb#image-2",
  masks: "model.glb#image-1"
};
var dimensions = getVirtualTextureDimensions(header, paths.albedo);
var materialSet = store.loadMaterialSet(paths, { ...dimensions, mipTail: true });
function loadIndependentTexture(path) {
  const existing = store.getEntry(path);
  if (existing)
    return existing;
  const size = getVirtualTextureDimensions(header, path);
  store.loadTexture(path, { ...size, mipTail: true });
  return store.getEntry(path);
}
var secondDimensions = getVirtualTextureDimensions(header, "model-2.glb#image-0");
var feedbackPasses = Array.from({ length: 4 }, () => new VT.VirtualTextureFeedbackPass(0.125));
var feedbackPass = feedbackPasses[0];
for (const pass of feedbackPasses)
  pass.resize(renderer.domElement.width, renderer.domElement.height);
var gltf = modelHandle.asset;
var model = gltf.scene;
var skinnedMeshes = [];
var sourceMaterial = null;
model.traverse((object) => {
  if (!object.isMesh)
    return;
  object.layers.set(MODEL_LAYER);
  object.castShadow = true;
  object.receiveShadow = true;
  if (!object.isSkinnedMesh)
    throw new Error(`rigged VT demo found a non-skinned render mesh: ${object.name}`);
  if (Array.isArray(object.material))
    throw new Error("rigged VT demo requires one material per primitive");
  sourceMaterial ??= object.material;
  skinnedMeshes.push(object);
});
if (skinnedMeshes.length === 0)
  throw new Error("model contains no SkinnedMesh");
if (gltf.animations.length === 0)
  throw new Error("model contains no animation clips");
var deformedBounds = new THREE.Box3;
var deformedVertex = new THREE.Vector3;
function measureDeformedBounds() {
  deformedBounds.makeEmpty();
  modelPivot.updateMatrixWorld(true);
  for (const mesh of skinnedMeshes) {
    const count = mesh.geometry.getAttribute("position").count;
    for (let index = 0;index < count; index++) {
      mesh.getVertexPosition(index, deformedVertex);
      deformedVertex.applyMatrix4(mesh.matrixWorld);
      deformedBounds.expandByPoint(deformedVertex);
    }
  }
  return deformedBounds;
}
var modelPivot = new THREE.Group;
modelPivot.add(model);
var box = new THREE.Box3().setFromObject(model);
var size = box.getSize(new THREE.Vector3);
modelPivot.scale.setScalar(2.55 / size.y);
box.setFromObject(modelPivot);
var center = box.getCenter(new THREE.Vector3);
modelPivot.position.set(-center.x, -box.min.y, -center.z);
scene.add(modelPivot);
var pair = VT.createVirtualGltfMaterialPair(THREE, store, materialSet, feedbackPass.pixelScale, {
  addressMode: VT.VirtualTextureAddressMode.Repeat,
  qualityBias: 0,
  baseColorFactor: [sourceMaterial.color.r, sourceMaterial.color.g, sourceMaterial.color.b, sourceMaterial.opacity],
  roughnessFactor: sourceMaterial.roughness,
  metalnessFactor: sourceMaterial.metalness,
  normalScale: [1, -1],
  side: sourceMaterial.side
});
var visibleMaterials = skinnedMeshes.map((mesh) => mesh.material);
for (let index = 0;index < skinnedMeshes.length; index++) {
  visibleMaterials[index] = pair.material;
  skinnedMeshes[index].material = pair.material;
}
var importedTextures = new Set;
for (const property of ["map", "normalMap", "roughnessMap", "metalnessMap", "aoMap", "emissiveMap"]) {
  const imported = sourceMaterial[property];
  if (imported)
    importedTextures.add(imported);
}
for (const imported of importedTextures) {
  imported.dispose();
  imported.source?.data?.close?.();
}
sourceMaterial.dispose();
function useFeedbackMaterial(enabled) {
  for (let index = 0;index < skinnedMeshes.length; index++)
    skinnedMeshes[index].material = enabled ? pair.feedbackMaterial : visibleMaterials[index];
}
var mixer = new THREE.AnimationMixer(model);
var action = mixer.clipAction(gltf.animations[0]);
action.play();
mixer.setTime(0);
modelPivot.updateMatrixWorld(true);
var animatedBounds = measureDeformedBounds();
modelPivot.position.y -= animatedBounds.min.y;
modelPivot.updateMatrixWorld(true);
animatedBounds = measureDeformedBounds();
var groundedMinY = animatedBounds.min.y;
var skeleton = new THREE.SkeletonHelper(model);
skeleton.visible = false;
scene.add(skeleton);
var secondGltf = secondModelHandle.asset;
var secondModel = secondGltf.scene;
var secondMeshes = [];
secondModel.traverse((object) => {
  if (!object.isMesh)
    return;
  object.layers.set(MODEL_LAYER);
  object.castShadow = true;
  object.receiveShadow = true;
  if (Array.isArray(object.material))
    throw new Error("model 2 requires one material per primitive");
  secondMeshes.push(object);
});
if (secondMeshes.length === 0)
  throw new Error("model 2 contains no render meshes");
var secondSkinnedMeshes = secondMeshes.filter((mesh) => mesh.isSkinnedMesh);
var secondPivot = new THREE.Group;
secondPivot.add(secondModel);
var secondBox = new THREE.Box3().setFromObject(secondModel);
var secondSize = secondBox.getSize(new THREE.Vector3);
secondPivot.scale.setScalar(2.55 / secondSize.y);
secondBox.setFromObject(secondPivot);
var secondCenter = secondBox.getCenter(new THREE.Vector3);
secondPivot.position.set(-secondCenter.x, -secondBox.min.y, -secondCenter.z);
scene.add(secondPivot);
var secondLayouts = new Map(secondGltf.materialTextures.map((layout) => [layout.name, layout]));
var secondRecords = [];
var secondImportedTextures = new Set;
var replacedSourceMaterials = new Set;
var secondMaxFeedbackChannels = 1;
for (const mesh of secondMeshes) {
  const source = mesh.material;
  const layout = secondLayouts.get(source.name);
  if (!layout || layout.baseColorImage === null) {
    secondRecords.push({ mesh, pair: null, visibleMaterial: source, wasVisible: mesh.visible });
    continue;
  }
  const entry = (image) => image === null ? undefined : loadIndependentTexture(`model-2.glb#image-${image}`);
  const set = {
    albedo: entry(layout.baseColorImage),
    normal: entry(layout.normalImage),
    masks: entry(layout.metallicRoughnessImage),
    emissive: entry(layout.emissiveImage)
  };
  const pair2 = VT.createVirtualGltfMaterialPair(THREE, store, set, feedbackPass.pixelScale, {
    addressMode: VT.VirtualTextureAddressMode.Repeat,
    qualityBias: 0,
    baseColorFactor: [source.color.r, source.color.g, source.color.b, source.opacity],
    roughnessFactor: source.roughness,
    metalnessFactor: source.metalness,
    normalScale: [1, -1],
    emissiveFactor: [source.emissive.r, source.emissive.g, source.emissive.b],
    transparent: source.transparent,
    depthWrite: source.depthWrite,
    side: source.side
  });
  secondMaxFeedbackChannels = Math.max(secondMaxFeedbackChannels, pair2.feedbackMaterials.length);
  mesh.material = pair2.material;
  secondRecords.push({ mesh, pair: pair2, visibleMaterial: pair2.material, wasVisible: mesh.visible });
  replacedSourceMaterials.add(source);
  for (const property of ["map", "normalMap", "roughnessMap", "metalnessMap", "aoMap", "emissiveMap"]) {
    const imported = source[property];
    if (imported)
      secondImportedTextures.add(imported);
  }
}
for (const imported of secondImportedTextures) {
  imported.dispose();
  imported.source?.data?.close?.();
}
for (const material of replacedSourceMaterials)
  material.dispose();
var secondClip = secondGltf.animations.find((clip) => clip.name === "Idle") ?? secondGltf.animations[0];
var secondMixer = new THREE.AnimationMixer(secondModel);
var secondAction = secondMixer.clipAction(secondClip);
secondAction.play();
secondMixer.setTime(0);
var secondSkeleton = new THREE.SkeletonHelper(secondModel);
secondSkeleton.visible = false;
scene.add(secondSkeleton);
var secondVertex = new THREE.Vector3;
var secondBounds = new THREE.Box3;
function measureSecondBounds() {
  secondBounds.makeEmpty();
  secondPivot.updateMatrixWorld(true);
  for (const mesh of secondMeshes)
    for (let index = 0;index < mesh.geometry.getAttribute("position").count; index++) {
      mesh.getVertexPosition(index, secondVertex);
      secondVertex.applyMatrix4(mesh.matrixWorld);
      secondBounds.expandByPoint(secondVertex);
    }
  return secondBounds;
}
var measuredSecond = measureSecondBounds();
secondPivot.position.y -= measuredSecond.min.y;
secondPivot.updateMatrixWorld(true);
measuredSecond = measureSecondBounds();
var secondGroundedMinY = measuredSecond.min.y;
var activeModel = 0;
var skeletonRequested = false;
secondPivot.visible = false;
function setActiveModel(modelNumber) {
  activeModel = modelNumber === 2 ? 1 : 0;
  modelPivot.visible = activeModel === 0;
  secondPivot.visible = activeModel === 1;
  skeleton.visible = activeModel === 0 && skeletonRequested;
  secondSkeleton.visible = activeModel === 1 && skeletonRequested;
  for (let index = 0;index < feedbackResults.length; index++)
    feedbackResults[index] = null;
  orbitAngle = 0;
  orbitVelocity = 0;
  cameraDistance = 4.1;
  zoomVelocity = 0;
}
function useSecondFeedbackMaterial(index, enabled) {
  for (const record of secondRecords) {
    if (!record.pair) {
      record.mesh.visible = enabled ? false : record.wasVisible;
      continue;
    }
    const feedbackIndex = Math.min(index, record.pair.feedbackMaterials.length - 1);
    record.mesh.material = enabled ? record.pair.feedbackMaterials[feedbackIndex] : record.visibleMaterial;
  }
}
await warmRendererVariants(renderer, [{ scene, camera }]);
skeleton.visible = true;
await warmRendererVariants(renderer, [{ scene, camera }]);
skeleton.visible = false;
modelPivot.visible = false;
secondPivot.visible = true;
await warmRendererVariants(renderer, [{ scene, camera }]);
renderer.render(scene, camera);
secondSkeleton.visible = true;
await warmRendererVariants(renderer, [{ scene, camera }]);
renderer.render(scene, camera);
secondSkeleton.visible = false;
var previousTarget = renderer.getRenderTarget();
var previousMask = camera.layers.mask;
var previousShadows = renderer.shadowMap.enabled;
renderer.shadowMap.enabled = false;
camera.layers.set(MODEL_LAYER);
modelPivot.visible = true;
secondPivot.visible = false;
useFeedbackMaterial(true);
renderer.setRenderTarget(feedbackPass.target);
await warmRendererVariants(renderer, [{ scene, camera }]);
renderer.render(scene, camera);
useFeedbackMaterial(false);
modelPivot.visible = false;
secondPivot.visible = true;
for (let index = 0;index < secondMaxFeedbackChannels; index++) {
  useSecondFeedbackMaterial(index, true);
  renderer.setRenderTarget(feedbackPasses[index].target);
  await warmRendererVariants(renderer, [{ scene, camera }]);
  renderer.render(scene, camera);
  useSecondFeedbackMaterial(index, false);
}
renderer.setRenderTarget(previousTarget);
camera.layers.mask = previousMask;
renderer.shadowMap.enabled = previousShadows;
modelPivot.visible = true;
secondPivot.visible = false;
renderer.render(scene, camera);
store.attachRenderer(renderer);
rendererSeal.seal();
function submitFeedback() {
  const mask = camera.layers.mask;
  const shadows = renderer.shadowMap.enabled;
  renderer.shadowMap.enabled = false;
  camera.layers.set(MODEL_LAYER);
  if (activeModel === 0) {
    useFeedbackMaterial(true);
    feedbackPass.submit(renderer, scene, camera, store);
    useFeedbackMaterial(false);
  } else {
    for (let index = 0;index < secondMaxFeedbackChannels; index++) {
      useSecondFeedbackMaterial(index, true);
      feedbackPasses[index].submit(renderer, scene, camera, store);
      useSecondFeedbackMaterial(index, false);
    }
  }
  camera.layers.mask = mask;
  renderer.shadowMap.enabled = shadows;
}
function setAnimationEnabled(enabled) {
  animationEnabled = Boolean(enabled);
  action.paused = !animationEnabled;
  secondAction.paused = !animationEnabled;
}
function setSkeletonVisible(visible) {
  skeletonRequested = Boolean(visible);
  skeleton.visible = activeModel === 0 && skeletonRequested;
  secondSkeleton.visible = activeModel === 1 && skeletonRequested;
}
var hud = document.getElementById("hud");
renderer.setAnimationLoop((now) => {
  const dt = Math.min(0.05, (now - last) / 1000);
  last = now;
  smoothedDt = smoothedDt * 0.95 + dt * 0.05;
  store.recordFrameTime(dt * 1000);
  if (animationEnabled) {
    mixer.update(dt);
    secondMixer.update(dt);
  }
  const rotateInput = programmatic ? 0 : (keys.has("d") ? 1 : 0) - (keys.has("a") ? 1 : 0);
  const zoomInput = programmatic ? 0 : (keys.has("s") ? 1 : 0) - (keys.has("w") ? 1 : 0);
  orbitVelocity += rotateInput * 7.5 * dt;
  zoomVelocity += zoomInput * 8 * dt;
  const damping = Math.exp(-7 * dt);
  orbitVelocity *= damping;
  zoomVelocity *= damping;
  orbitAngle += orbitVelocity * dt;
  cameraDistance = Math.max(1.35, Math.min(8, cameraDistance + zoomVelocity * dt));
  if (cameraDistance === 1.35 && zoomVelocity < 0 || cameraDistance === 8 && zoomVelocity > 0)
    zoomVelocity = 0;
  camera.position.set(Math.sin(orbitAngle) * cameraDistance, 1.45, Math.cos(orbitAngle) * cameraDistance);
  camera.lookAt(0, 1.25, 0);
  const expectedFeedback = activeModel === 0 ? 1 : secondMaxFeedbackChannels;
  for (let index = 0;index < feedbackPasses.length; index++) {
    const completed = feedbackPasses[index].consume();
    if (completed && index < expectedFeedback)
      feedbackResults[index] = completed;
  }
  let feedbackBatchReady = true;
  for (let index = 0;index < expectedFeedback; index++)
    if (feedbackResults[index] === null)
      feedbackBatchReady = false;
  if (feedbackBatchReady) {
    mergedFeedback.clear();
    for (let index = 0;index < expectedFeedback; index++) {
      for (const [key2, request] of feedbackResults[index])
        mergedFeedback.set(key2, request);
      feedbackResults[index] = null;
    }
    lastResult = store.processFeedback(mergedFeedback);
  }
  store.poll();
  renderer.render(scene, camera);
  if (feedbackEnabled && frame % FEEDBACK_INTERVAL === 0)
    submitFeedback();
  frame++;
  for (let index = waiters.length - 1;index >= 0; index--) {
    if (frame < waiters[index].target)
      continue;
    waiters[index].resolve();
    waiters.splice(index, 1);
  }
  if (frame % 15 === 0) {
    const stats = store.getStats();
    const activeGltf = activeModel === 0 ? gltf : secondGltf;
    const activeDimensions = activeModel === 0 ? dimensions : secondDimensions;
    const activeMeshes = activeModel === 0 ? skinnedMeshes : secondMeshes;
    const activeMips = activeModel === 0 ? feedbackPass.getLatestMips() : feedbackPasses.slice(0, secondMaxFeedbackChannels).flatMap((pass) => pass.getLatestMips());
    hud.innerHTML = `<b>afterglow — Model VT</b> · model ${activeModel + 1}/2<br>` + `Meshes: ${activeMeshes.length} · ${activeModel === 0 ? skinnedMeshes.length : secondSkinnedMeshes.length} skinned<br>` + `Animation: ${activeModel === 0 ? gltf.animations[0].name : secondClip.name} · ${(activeModel === 0 ? gltf.animations[0].duration : secondClip.duration).toFixed(2)} s · ${animationEnabled ? "playing" : "paused"}<br>` + `Pipeline: GLB from .big · runtime meshopt ACMR ${activeGltf.meshOptimization[0].originalAcmr.toFixed(2)}→${activeGltf.meshOptimization[0].optimizedAcmr.toFixed(2)}<br>` + `Material: extracted glTF channels through VT<br>` + `Base color: ${activeDimensions.width}×${activeDimensions.height} · atlas: ${store.atlasWidth}² · ${format === 0 ? "BC7" : format === 1 ? "ASTC" : "RGBA"}<br>` + `Resident: ${stats.atlasSlotsUsed}/${stats.atlasSlotsTotal} · pending: ${stats.pendingPages} · requests: ${lastResult.totalRequests}<br>` + `Feedback mips: [${activeMips.join(",")}] · FPS: ${(1 / smoothedDt).toFixed(0)} · errors: ${errors.length}`;
  }
});
addEventListener("resize", () => {
  camera.aspect = innerWidth / innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(innerWidth, innerHeight);
  for (const pass of feedbackPasses)
    pass.resize(renderer.domElement.width, renderer.domElement.height);
});
addEventListener("keydown", (event) => {
  if (programmatic)
    return;
  const keyName = event.key.toLowerCase();
  keys.add(keyName);
  if (event.repeat)
    return;
  if (keyName === "1")
    setActiveModel(1);
  else if (keyName === "2")
    setActiveModel(2);
  else if (event.code === "Space")
    setAnimationEnabled(!animationEnabled);
  else if (keyName === "b")
    setSkeletonVisible(!skeletonRequested);
  else if (keyName === "f")
    feedbackEnabled = !feedbackEnabled;
  else if (keyName === "r") {
    keys.clear();
    orbitAngle = 0;
    orbitVelocity = 0;
    cameraDistance = 4.1;
    zoomVelocity = 0;
  }
});
addEventListener("keyup", (event) => keys.delete(event.key.toLowerCase()));
addEventListener("blur", () => keys.clear());
window.__afterglowRiggedVT = {
  setProgrammatic(enabled) {
    programmatic = Boolean(enabled);
  },
  setAnimationEnabled,
  setAnimationTime(seconds) {
    if (activeModel === 0)
      mixer.setTime(Math.max(0, seconds) % gltf.animations[0].duration);
    else
      secondMixer.setTime(Math.max(0, seconds) % secondClip.duration);
  },
  measureBounds() {
    const bounds = activeModel === 0 ? measureDeformedBounds() : measureSecondBounds();
    return { minY: bounds.min.y, maxY: bounds.max.y };
  },
  setActiveModel,
  setFeedbackEnabled(enabled) {
    feedbackEnabled = Boolean(enabled);
  },
  setSkeletonVisible,
  setView(angle, distance) {
    orbitAngle = angle;
    orbitVelocity = 0;
    cameraDistance = Math.max(1.35, Math.min(8, distance));
    zoomVelocity = 0;
  },
  step(count = 1) {
    return new Promise((resolve) => waiters.push({ target: frame + count, resolve }));
  },
  telemetry: () => store.getStats(),
  debugSnapshot: () => store.getDebugSnapshot(),
  feedbackMips: () => feedbackPass.getLatestMips(),
  errorCount: () => errors.length,
  errors: () => errors.slice(),
  status: () => {
    const activeGltf = activeModel === 0 ? gltf : secondGltf;
    const activeMeshes = activeModel === 0 ? skinnedMeshes : secondMeshes;
    const activeDimensions = activeModel === 0 ? dimensions : secondDimensions;
    const optimization = activeGltf.meshOptimization[0];
    return {
      activeModel: activeModel + 1,
      meshes: activeMeshes.length,
      skinnedMeshes: activeModel === 0 ? skinnedMeshes.length : secondSkinnedMeshes.length,
      bones: activeModel === 0 ? skinnedMeshes[0].skeleton.bones.length : secondSkinnedMeshes[0]?.skeleton.bones.length ?? 0,
      clip: activeModel === 0 ? gltf.animations[0].name : secondClip.name,
      clipDuration: activeModel === 0 ? gltf.animations[0].duration : secondClip.duration,
      animationEnabled,
      feedbackEnabled,
      skeletonVisible: skeleton.visible,
      groundedMinY: activeModel === 0 ? groundedMinY : secondGroundedMinY,
      orbitAngle,
      cameraDistance,
      sourceWidth: activeDimensions.width,
      sourceHeight: activeDimensions.height,
      material: "virtual-gltf-metallic-roughness",
      packedAsset: activeModel === 0 ? "model.glb" : "model-2.glb",
      meshOptimized: activeGltf.meshOptimization.length === activeMeshes.length,
      originalAcmr: optimization.originalAcmr,
      optimizedAcmr: optimization.optimizedAcmr,
      preservedAttributes: optimization.preservedAttributes,
      sameMeshFeedback: true,
      feedbackChannels: activeModel === 0 ? 1 : secondMaxFeedbackChannels,
      shadows: renderer.shadowMap.enabled && key.castShadow && floor.receiveShadow,
      shadowMapSize: key.shadow.mapSize.x,
      rendererSealed: rendererSeal.isSealed,
      pipelineViolations: rendererSeal.violations
    };
  }
};
