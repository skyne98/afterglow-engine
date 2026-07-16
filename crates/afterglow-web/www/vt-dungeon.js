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
  return {
    async load(path) {
      const response = await fetch(url(path));
      if (!response.ok)
        throw new Error(`asset fetch ${response.status}: ${path}`);
      return new Uint8Array(await response.arrayBuffer());
    },
    async size(path) {
      const response = await fetch(url(path), { method: "HEAD" });
      if (!response.ok)
        throw new Error(`asset HEAD ${response.status}: ${path}`);
      const length = Number(response.headers.get("content-length"));
      if (!Number.isSafeInteger(length) || length < 0)
        throw new Error(`asset HEAD has invalid content-length: ${path}`);
      return length;
    },
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
function createPageDataProvider(loader, header, textureWorkers, format) {
  const directories = new Map;
  for (const asset of header.assets) {
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
    maxTranscodeMs: 0
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
    return transcoded.slice(16, 16 + length);
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
async function createWebGPUOnlyRenderer(parameters = {}) {
  const gpu = navigator.gpu;
  if (!gpu)
    throw new Error("navigator.gpu is unavailable. WebGL fallback is disabled.");
  const adapter = await gpu.requestAdapter();
  if (!adapter)
    throw new Error("Unable to acquire a hardware WebGPU adapter. WebGL fallback is disabled.");
  const renderer = new window.THREE.WebGPURenderer({ ...parameters });
  disableWebGLFallback(renderer);
  try {
    await renderer.init();
    assertWebGPUBackend(renderer);
  } catch (error) {
    renderer.dispose?.();
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

// crates/afterglow-web/www/vt-dungeon.ts
var THREE = window.THREE;
var VT = window.AfterglowVT;
if (!VT)
  throw new Error("AfterglowVT engine bundle is unavailable");
var { wgslFn, Fn, texture, sampler, uv, uniform, float, uint } = THREE;
var VT_LOD_BIAS = -1.5;
var FEEDBACK_INTERVAL = 4;
var scene = new THREE.Scene;
scene.background = new THREE.Color(1053464);
scene.fog = new THREE.Fog(1053464, 7, 28);
var camera = new THREE.PerspectiveCamera(70, innerWidth / innerHeight, 0.05, 60);
camera.rotation.order = "YXZ";
var renderer = await createWebGPUOnlyRenderer({ antialias: false, trackTimestamp: true }).catch((error) => {
  showWebGPUFailure(error);
  throw error;
});
renderer.setSize(innerWidth, innerHeight);
renderer.setPixelRatio(devicePixelRatio);
document.body.append(renderer.domElement);
var rendererSeal = new RendererSeal(renderer.backend);
var rendererSealStats = { renderPipelines: 0, computePipelines: 0, renderPipelineViolations: 0, computePipelineViolations: 0 };
function pipelineTelemetry() {
  rendererSealStats.renderPipelines = rendererSeal.renderPipelines;
  rendererSealStats.computePipelines = rendererSeal.computePipelines;
  rendererSealStats.renderPipelineViolations = rendererSeal.renderPipelineViolations;
  rendererSealStats.computePipelineViolations = rendererSeal.computePipelineViolations;
  return rendererSealStats;
}
var errors = [];
renderer.backend.device.addEventListener("uncapturederror", (e) => errors.push(String(e.error?.message ?? e.error)));
addEventListener("error", (e) => errors.push(String(e.error?.stack ?? e.message)));
addEventListener("unhandledrejection", (e) => errors.push(String(e.reason?.stack ?? e.reason)));
scene.add(new THREE.HemisphereLight(12175592, 2366229, 1.6));
var lamp = new THREE.PointLight(16763269, 30, 18, 2);
lamp.position.set(0, 3.2, 0);
scene.add(lamp);
var floor = new THREE.Mesh(new THREE.PlaneGeometry(16, 16), new THREE.MeshStandardMaterial({ color: 2696994, roughness: 1 }));
floor.rotation.x = -Math.PI / 2;
scene.add(floor);
var ceiling = floor.clone();
ceiling.position.y = 4;
ceiling.rotation.x = Math.PI / 2;
ceiling.material = new THREE.MeshStandardMaterial({ color: 1579291, roughness: 1 });
scene.add(ceiling);
var rangeLoader = createFetchRangeLoader();
var TEXTURE_WORKER_COUNT = Math.max(2, Math.min(4, Math.floor((navigator.hardwareConcurrency || 4) / 2)));
var textureRpcs = await Promise.all(Array.from({ length: TEXTURE_WORKER_COUNT }, () => Rpc.create({
  mainWasmUrl: "afterglow_web.wasm",
  workerJsUrl: "worker.js",
  workerWasmUrl: "texture.wasm",
  timeoutMs: 1e4
})));
var textureWorkers = textureRpcs.map((rpc) => new TextureClient(rpc));
addEventListener("beforeunload", () => {
  for (const rpc of textureRpcs)
    rpc.terminate();
}, { once: true });
var prefix = await rangeLoader.read("vt-dungeon.big", 0, 16);
var dataOffset = Number(new DataView(prefix.buffer, prefix.byteOffset + 8, 8).getBigUint64(0, true));
var headerBytes = await rangeLoader.read("vt-dungeon.big", 0, dataOffset);
var { header } = parseBigHeader(headerBytes);
var format = renderer.backend.device.features.has("texture-compression-bc") ? 0 : renderer.backend.device.features.has("texture-compression-astc") ? 1 : VT.FORMAT_RGBA;
var containerLoader = {
  load: (path) => rangeLoader.load(path),
  size: (path) => rangeLoader.size(path),
  read: (_path, offset, len) => rangeLoader.read("vt-dungeon.big", offset, len)
};
var pageProvider = createPageDataProvider(containerLoader, header, textureWorkers, format);
var loader = { read: (path, offset, len) => rangeLoader.read(path, offset, len), poll() {} };
var vtTuning = new VT.VirtualTextureTuning;
var store = new VT.VirtualTextureStore(loader, pageProvider, format, renderer.backend.device, vtTuning);
var vtSampleLevel = wgslFn(VT.VT_SAMPLE_LEVEL_WGSL);
var vtResolveMaterialMip4 = wgslFn(VT.VT_RESOLVE_MATERIAL_MIP4_WGSL);
var vtFeedback = wgslFn(VT.VT_FEEDBACK_WGSL);
var atlasNode = texture(store.atlasTexture);
var atlasSampler = sampler(atlasNode);
var feedbackScene = new THREE.Scene;
var feedbackPass = new VT.VirtualTextureFeedbackPass(0.125);
var materialNames = ["Rock064", "Ground103", "PavingStones150"];
var materialSets = materialNames.map((name) => {
  const paths = { albedo: `${name}_Color.png`, normal: `${name}_NormalGL.png`, masks: `${name}_Masks.png` };
  const dimensions = getVirtualTextureDimensions(header, paths.albedo);
  return store.loadMaterialSet(paths, { ...dimensions, mipTail: true });
});
var segments = [
  [-8, -8, 8, -8],
  [8, -8, 8, 8],
  [8, 8, -8, 8],
  [-8, 8, -8, -8],
  [-3, -8, -3, 1],
  [-3, 1, 2, 1],
  [2, 1, 2, 8],
  [3, -8, 3, -1],
  [-2, -1, 3, -1],
  [-2, -1, -2, 5],
  [-2, 5, 4, 5],
  [4, 5, 4, 8]
];
var walls = [];
function feedbackMaterial(entry) {
  const material = new THREE.MeshBasicNodeMaterial({ side: THREE.DoubleSide });
  material.fragmentNode = Fn(() => vtFeedback({ uv: uv().fract(), virtualSize: uniform(new THREE.Vector2(entry.width, entry.height)), pageGrid: uniform(new THREE.Vector2(entry.pageGridX, entry.pageGridY)), maxMip: float(entry.maxMip), lodBias: float(VT_LOD_BIAS), textureId: uint(entry.textureId) }))();
  return material;
}
function sampleEntryAtMip(entry, resolvedMip) {
  const pageTable = texture(entry.pageTableTexture);
  return vtSampleLevel({ pageTable, atlas: atlasNode, atlasSampler, uv: uv(), virtualSize: uniform(new THREE.Vector2(entry.width, entry.height)), pageGrid: uniform(new THREE.Vector2(entry.pageGridX, entry.pageGridY)), pageSize: float(VT.PAGE_SIZE), pageBorder: float(VT.PAGE_BORDER), atlasSize: uniform(new THREE.Vector2(store.atlasWidth, store.atlasHeight)), maxMip: float(entry.maxMip), resolvedMip, addressMode: uint(1) });
}
function wallMaterial(set) {
  if (!set.normal || !set.masks)
    throw new Error("dungeon PBR material set requires albedo, normal, and packed masks");
  const material = new THREE.MeshStandardNodeMaterial({ metalness: 0, side: THREE.DoubleSide });
  const resolvedMip = Fn(() => vtResolveMaterialMip4({ pageTable0: texture(set.albedo.pageTableTexture), pageTable1: texture(set.normal.pageTableTexture), pageTable2: texture(set.masks.pageTableTexture), pageTable3: texture(set.masks.pageTableTexture), uv: uv(), virtualSize: uniform(new THREE.Vector2(set.albedo.width, set.albedo.height)), pageGrid: uniform(new THREE.Vector2(set.albedo.pageGridX, set.albedo.pageGridY)), pageSize: float(VT.PAGE_SIZE), maxMip: float(set.albedo.maxMip), textureMaxMip: float(set.albedo.textureMaxMip), addressMode: uint(1) }))().toVar();
  material.colorNode = Fn(() => {
    const color = sampleEntryAtMip(set.albedo, resolvedMip);
    return THREE.vec4(THREE.sRGBTransferEOTF(color.rgb), color.a);
  })();
  const masks = Fn(() => sampleEntryAtMip(set.masks, resolvedMip))().toVar();
  material.normalNode = Fn(() => THREE.normalMap(sampleEntryAtMip(set.normal, resolvedMip).xyz))();
  material.roughnessNode = Fn(() => masks.r)();
  material.aoNode = Fn(() => masks.g)();
  return material;
}
for (let i = 0;i < segments.length; i++) {
  const [x1, z1, x2, z2] = segments[i], set = materialSets[i % materialSets.length], entry = set.albedo, path = entry.path;
  const dx = x2 - x1, dz = z2 - z1, len = Math.hypot(dx, dz), geometry = new THREE.PlaneGeometry(len, 4, 1, 1);
  for (let u = 0;u < geometry.attributes.uv.count; u++)
    geometry.attributes.uv.setX(u, geometry.attributes.uv.getX(u) * len / 4);
  const mesh = new THREE.Mesh(geometry, wallMaterial(set));
  mesh.position.set((x1 + x2) / 2, 2, (z1 + z2) / 2);
  mesh.rotation.y = Math.atan2(-dz, dx);
  scene.add(mesh);
  const feedbackMesh = new THREE.Mesh(geometry, feedbackMaterial(entry));
  feedbackMesh.position.copy(mesh.position);
  feedbackMesh.rotation.copy(mesh.rotation);
  feedbackScene.add(feedbackMesh);
  walls.push({ path, entry, x1, z1, x2, z2, len, mesh, feedbackMesh });
}
var PLAYER_RADIUS = 0.28;
var pose = { x: -5.5, z: -5.5, yaw: 0, pitch: 0 };
var keys = new Set;
var programmatic = false;
var diagnosticAtlas = false;
var last = performance.now();
var smoothedDt = 1 / 60;
var frame = 0;
var lastResult = { loaded: 0, evicted: 0, totalRequests: 0, lodBias: 0 };
var runtimeTiming = { vtCpuUs: 0, renderSubmitUs: 0, feedbackSubmitUs: 0, frameCpuUs: 0, gpuMainMs: 0, gpuFeedbackMs: 0, gpuTotalMs: 0, gpuTimestampSupported: Boolean(renderer.backend.hasTimestamp) };
var resolvingGpuTimestamps = false;
async function resolveGpuTimings() {
  if (!runtimeTiming.gpuTimestampSupported || resolvingGpuTimestamps)
    return runtimeTiming;
  resolvingGpuTimestamps = true;
  try {
    runtimeTiming.gpuTotalMs = await renderer.resolveTimestampsAsync("render");
    const contexts = renderer._renderContexts, pool = renderer.backend.timestampQueryPool?.render, timestamps = pool?.timestamps;
    if (contexts && timestamps) {
      const mainContext = contexts.get(null).id, feedbackContext = contexts.get(feedbackPass.target).id;
      let mainFrame = -1, feedbackFrame = -1;
      for (const [uid, duration] of timestamps) {
        const parts = uid.split(":"), context = Number(parts[2]), id = Number(parts[3]?.slice(1));
        if (context === mainContext && id > mainFrame) {
          mainFrame = id;
          runtimeTiming.gpuMainMs = duration;
        } else if (context === feedbackContext && id > feedbackFrame) {
          feedbackFrame = id;
          runtimeTiming.gpuFeedbackMs = duration;
        }
      }
      timestamps.clear();
      const frames = pool.getTimestampFrames?.();
      if (frames)
        frames.length = 0;
    }
  } finally {
    resolvingGpuTimestamps = false;
  }
  return runtimeTiming;
}
function setGpuTimingEnabled(enabled) {
  const active = Boolean(enabled) && runtimeTiming.gpuTimestampSupported;
  renderer.backend.trackTimestamp = active;
  for (const pool of Object.values(renderer.backend.timestampQueryPool ?? {}))
    if (pool)
      pool.trackTimestamp = active;
}
function pointSegmentDistance(x, z, s) {
  const dx = s.x2 - s.x1, dz = s.z2 - s.z1, l2 = dx * dx + dz * dz, t = Math.max(0, Math.min(1, ((x - s.x1) * dx + (z - s.z1) * dz) / l2)), px = s.x1 + t * dx, pz = s.z1 + t * dz;
  return Math.hypot(x - px, z - pz);
}
function valid(x, z) {
  return x > -7.7 && x < 7.7 && z > -7.7 && z < 7.7 && walls.slice(4).every((s) => pointSegmentDistance(x, z, s) > PLAYER_RADIUS);
}
function setPose(x, z, yaw = pose.yaw, pitch = pose.pitch) {
  if (valid(x, z)) {
    pose.x = x;
    pose.z = z;
  }
  pose.yaw = yaw;
  pose.pitch = Math.max(-1.45, Math.min(1.45, pitch));
}
function move(forward, strafe) {
  const sin = Math.sin(pose.yaw), cos = Math.cos(pose.yaw), dx = -sin * forward + cos * strafe, dz = -cos * forward - sin * strafe;
  if (valid(pose.x + dx, pose.z))
    pose.x += dx;
  if (valid(pose.x, pose.z + dz))
    pose.z += dz;
}
function update(dt) {
  let f = (keys.has("w") ? 1 : 0) - (keys.has("s") ? 1 : 0), s = (keys.has("d") ? 1 : 0) - (keys.has("a") ? 1 : 0), sprint = keys.has("shift") ? 5.5 : 2.8;
  if (f || s) {
    const n = Math.hypot(f, s);
    move(f / n * sprint * dt, s / n * sprint * dt);
  }
  camera.position.set(pose.x, 1.7, pose.z);
  camera.rotation.set(pose.pitch, pose.yaw, 0);
  camera.updateMatrixWorld();
  lamp.position.set(pose.x, 3.1, pose.z);
  const stageStart = performance.now();
  const feedback = feedbackPass.consume();
  if (feedback && !diagnosticAtlas)
    lastResult = store.processFeedback(feedback);
  store.poll();
  runtimeTiming.vtCpuUs = (performance.now() - stageStart) * 1000;
}
feedbackPass.resize(renderer.domElement.width, renderer.domElement.height);
await warmRendererVariants(renderer, [{ scene, camera }]);
var previousTarget = renderer.getRenderTarget();
renderer.setRenderTarget(feedbackPass.target);
await warmRendererVariants(renderer, [{ scene: feedbackScene, camera }]);
renderer.setRenderTarget(previousTarget);
await new Promise((r) => setTimeout(r, 0));
renderer.render(scene, camera);
renderer.setRenderTarget(feedbackPass.target);
renderer.render(feedbackScene, camera);
renderer.setRenderTarget(previousTarget);
store.attachRenderer(renderer);
rendererSeal.seal();
var waiters = [];
var hud = document.getElementById("hud");
var hudVisible = true;
renderer.setAnimationLoop((now) => {
  const frameCpuStart = performance.now(), dt = Math.min(0.05, (now - last) / 1000);
  last = now;
  smoothedDt = smoothedDt * 0.95 + dt * 0.05;
  store.recordFrameTime(dt * 1000);
  update(dt);
  const renderStart = performance.now();
  renderer.render(scene, camera);
  runtimeTiming.renderSubmitUs = (performance.now() - renderStart) * 1000;
  if (!diagnosticAtlas && frame % FEEDBACK_INTERVAL === 0) {
    const feedbackStart = performance.now();
    feedbackPass.submit(renderer, feedbackScene, camera, store);
    runtimeTiming.feedbackSubmitUs = (performance.now() - feedbackStart) * 1000;
  } else
    runtimeTiming.feedbackSubmitUs = 0;
  runtimeTiming.frameCpuUs = (performance.now() - frameCpuStart) * 1000;
  frame++;
  for (let i = waiters.length - 1;i >= 0; i--)
    if (frame >= waiters[i].target) {
      waiters[i].resolve();
      waiters.splice(i, 1);
    }
  if (hudVisible && frame % 15 === 0) {
    const d = store.getStats();
    hud.innerHTML = `<b>afterglow — Engine VT Dungeon</b><br>3 × 8K scanned PBR material sets · 12 wall instances<br>Virtual RGBA channels: 1.875 GiB · physical atlas: ${store.atlasWidth}²<br>Position: ${pose.x.toFixed(2)}, ${pose.z.toFixed(2)} · yaw ${(pose.yaw * 180 / Math.PI).toFixed(0)}° · ${(1 / smoothedDt).toFixed(0)} FPS<br>Textures: ${d.textureCount} · resident ${d.atlasSlotsUsed}/${d.atlasSlotsTotal} · pending ${d.pendingPages}<br>GPU feedback pages: ${lastResult.totalRequests} · mips [${feedbackPass.getLatestMips().join(",")}] · bias ${VT_LOD_BIAS} · budget ${d.budget} · errors ${errors.length}`;
  }
});
addEventListener("resize", () => {
  camera.aspect = innerWidth / innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(innerWidth, innerHeight);
  feedbackPass.resize(renderer.domElement.width, renderer.domElement.height);
});
addEventListener("keydown", (e) => {
  if (programmatic)
    return;
  keys.add(e.key.toLowerCase());
  if (e.key.toLowerCase() === "r")
    setPose(-5.5, -5.5, 0, 0);
  if (e.key === "1")
    setPose(-5.5, -5.5, 0, 0);
  if (e.key === "2")
    setPose(5.5, -5.5, Math.PI, 0);
  if (e.key === "3")
    setPose(5.5, 6.5, -Math.PI / 2, 0);
});
addEventListener("keyup", (e) => keys.delete(e.key.toLowerCase()));
renderer.domElement.addEventListener("click", () => {
  if (!programmatic)
    renderer.domElement.requestPointerLock();
});
addEventListener("mousemove", (e) => {
  if (!programmatic && document.pointerLockElement === renderer.domElement) {
    pose.yaw -= e.movementX * 0.002;
    pose.pitch = Math.max(-1.45, Math.min(1.45, pose.pitch - e.movementY * 0.002));
  }
});
var scenarios = { forward: () => setPose(-5.5, -5.5, 0, 0), reverse: () => setPose(5.5, -5.5, Math.PI, 0), corner: () => setPose(5.8, 6.4, -Math.PI / 2, -0.2) };
function atlasFeedback(groupCount, startPage = 0) {
  const feedback = new Map, albedos = materialSets.map((set) => set.albedo);
  for (let index = 0;index < groupCount; index++) {
    const entry = albedos[index % albedos.length], local = startPage + Math.floor(index / albedos.length), page = local % (entry.pageGridX * entry.pageGridY);
    feedback.set(index, { path: entry.path, mip: 0, x: page % entry.pageGridX, y: Math.floor(page / entry.pageGridX) });
  }
  return feedback;
}
async function waitForAtlas(target, timeout, feedback = null) {
  const end = performance.now() + timeout;
  let steps = 0;
  while (performance.now() < end) {
    const stats = store.getStats();
    if (stats.atlasSlotsUsed >= target && !stats.pendingPages && !stats.scheduledRequests && !stats.readyUploads)
      return true;
    if (feedback && steps % FEEDBACK_INTERVAL === 0)
      store.processFeedback(feedback);
    steps++;
    await window.__afterglowVtDungeon.step(1);
  }
  return false;
}
async function runAtlasScenario(name, timeout = 120000) {
  if (!["cold", "half", "full", "churn"].includes(name))
    throw new Error(`unknown atlas scenario ${name}`);
  const previousProgrammatic = programmatic;
  programmatic = true;
  diagnosticAtlas = true;
  keys.clear();
  feedbackPass.consume();
  try {
    const initial = store.getStats(), total = initial.atlasSlotsTotal;
    let target = name === "half" ? Math.floor(total / 2) : name === "cold" ? initial.atlasSlotsUsed : Math.floor(total * 0.995);
    if (name === "cold") {
      await waitForAtlas(target, timeout);
      target = store.getStats().atlasSlotsUsed;
    } else {
      const groups = Math.ceil(Math.max(0, target - initial.atlasSlotsUsed) / 3) + 32;
      const admission = atlasFeedback(groups, name === "half" ? 0 : 1024);
      store.processFeedback(admission);
      await waitForAtlas(target, timeout, admission);
    }
    if (name === "churn") {
      const before = store.getStats().cacheEvictions, groups = Math.ceil(total / 3);
      const replacement = atlasFeedback(groups, 3072);
      for (let epoch = 0;epoch < 17; epoch++)
        store.processFeedback(replacement);
      const end = performance.now() + timeout;
      let steps = 0;
      while (performance.now() < end && (store.getStats().cacheEvictions === before || store.getStats().pendingPages || store.getStats().scheduledRequests || store.getStats().readyUploads)) {
        if (steps % FEEDBACK_INTERVAL === 0)
          store.processFeedback(replacement);
        steps++;
        await window.__afterglowVtDungeon.step(1);
      }
    }
    return { name, target, ...store.getStats(), timing: { ...runtimeTiming }, errors: errors.length };
  } finally {
    diagnosticAtlas = false;
    programmatic = previousProgrammatic;
  }
}
window.__afterglowVtDungeon = {
  ready: () => true,
  telemetry: () => store.getStats(),
  timing: () => runtimeTiming,
  pipelineTelemetry,
  resolveGpuTimings,
  setGpuTimingEnabled,
  errorCount: () => errors.length,
  runAtlasScenario,
  snapshot: () => ({ pose: { ...pose }, ...store.getDebugSnapshot(), requests: lastResult.totalRequests, feedbackMips: [...feedbackPass.getLatestMips()], errors: [...errors] }),
  setProgrammatic: (enabled) => {
    programmatic = Boolean(enabled);
    keys.clear();
    if (programmatic && document.pointerLockElement)
      document.exitPointerLock();
  },
  setHudVisible: (visible) => {
    hudVisible = Boolean(visible);
    hud.style.display = hudVisible ? "" : "none";
  },
  setPose,
  getPose: () => ({ ...pose }),
  move,
  look: (yaw, pitch) => setPose(pose.x, pose.z, pose.yaw + yaw, pose.pitch + pitch),
  step: (n) => new Promise((resolve) => waiters.push({ target: frame + Math.max(1, n | 0), resolve })),
  waitForIdle: async (timeout = 5000) => {
    const end = performance.now() + timeout;
    while ((store.getStats().pendingPages || store.getStats().scheduledRequests || store.getStats().readyUploads) && performance.now() < end)
      await window.__afterglowVtDungeon.step(1);
    return store.getStats().pendingPages === 0 && store.getStats().scheduledRequests === 0 && store.getStats().readyUploads === 0;
  },
  runScenario: async (name) => {
    if (!scenarios[name])
      throw new Error(`unknown scenario ${name}`);
    programmatic = true;
    keys.clear();
    scenarios[name]();
    await window.__afterglowVtDungeon.step(90);
    await window.__afterglowVtDungeon.waitForIdle();
    return window.__afterglowVtDungeon.snapshot();
  }
};
