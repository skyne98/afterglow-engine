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

// crates/afterglow-web/web/src/workers/async-worker.ts
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

// crates/afterglow-web/web/src/workers/texture.client.ts
class TextureClient {
  rpc;
  closed = false;
  static async spawn(workerWasmUrl = "texture.wasm", baseUrl = "") {
    const driver = new AsyncWorker(null, baseUrl);
    const memory = new WebAssembly.Memory({ shared: true, initial: 256, maximum: 1024 });
    const { instance } = await WebAssembly.instantiate(await (await fetch(workerWasmUrl)).arrayBuffer(), asyncWorkerImports(driver, memory));
    driver.w = instance.exports;
    instance.exports.afterglow_wasm_init();
    return new TextureClient(driver);
  }
  static async spawnThreaded(opts = {}) {
    const rpc = await Rpc.create({
      mainWasmUrl: opts.mainWasmUrl ?? "afterglow_rpc.wasm",
      workerJsUrl: opts.workerJsUrl ?? "worker.js",
      workerWasmUrl: opts.workerWasmUrl ?? "texture.wasm",
      timeoutMs: opts.timeoutMs
    });
    return new TextureClient(rpc);
  }
  poll() {
    this.rpc.poll?.();
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
export {
  TextureClient
};
