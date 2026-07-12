// codec.js
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
  const msg = new TextDecoder().decode(bytes.subarray(eoff, eoff + mlen));
  throw new Error(`RPC ${variant === 1 ? "server" : "decode"} error (method ${method}): ${msg}`);
}

// async-worker.js
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
    const end = offset + len - 1;
    this.promise = fetch(url, { headers: { Range: `bytes=${offset}-${end}` } });
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
    this.pendingFetches = new Map;
    this._pendingCalls = new Map;
    this._taskIdCounter = 0;
  }
  async call(method, args) {
    const taskId = this.serveAsync(method, args);
    if (taskId < 0)
      throw new Error("async worker: serveAsync failed");
    return new Promise((resolve, reject) => {
      this._pendingCalls.set(taskId, { resolve, reject });
      const poll = () => {
        this.poll();
        if (this._pendingCalls.has(taskId)) {
          setTimeout(poll, 0);
        }
      };
      setTimeout(poll, 0);
    });
  }
  serveAsync(method, args) {
    const taskId = this._nextTaskId();
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
  poll() {
    this.w.afterglow_wasm_tick();
    const completions = [];
    const outPtr = this.w.afterglow_wasm_output_ptr();
    const outSize = this.w.afterglow_wasm_output_size();
    for (;; ) {
      const n = this.w.afterglow_wasm_drain_completion(outPtr, outSize);
      if (n < 0)
        break;
      const bytes = new Uint8Array((this._memory || this.w.memory).buffer, outPtr, n).slice();
      const dv = new DataView(bytes.buffer, bytes.byteOffset, 8);
      const taskId = Number(dv.getBigUint64(0, true));
      const responseBytes = bytes.subarray(8);
      completions.push({ taskId, response: responseBytes });
      const pending = this._pendingCalls.get(taskId);
      if (pending) {
        this._pendingCalls.delete(taskId);
        try {
          pending.resolve(unwrapResponse(responseBytes));
        } catch (e) {
          pending.reject(e);
        }
      }
    }
    return completions;
  }
  fetchStart(urlPtr, urlLen) {
    const url = new TextDecoder().decode(new Uint8Array((this._memory || this.w.memory).buffer, urlPtr, urlLen));
    const fullUrl = this._resolveUrl(url);
    const id = this.nextFetchId++;
    this.pendingFetches.set(id, new PendingFetch(fullUrl));
    return id;
  }
  fetchPoll(fetchId, outPtr, outMax) {
    const pending = this.pendingFetches.get(fetchId);
    if (!pending)
      return -1;
    if (!pending.resolved)
      return -1;
    this.pendingFetches.delete(fetchId);
    if (pending.error) {
      return 0;
    }
    if (pending.bytes.length > outMax)
      return -2;
    new Uint8Array((this._memory || this.w.memory).buffer, outPtr, outMax).set(pending.bytes);
    return pending.bytes.length;
  }
  headStart(urlPtr, urlLen) {
    const url = new TextDecoder().decode(new Uint8Array((this._memory || this.w.memory).buffer, urlPtr, urlLen));
    const fullUrl = this._resolveUrl(url);
    const id = this.nextFetchId++;
    this.pendingFetches.set(id, new HeadFetch(fullUrl));
    return id;
  }
  headPoll(fetchId, outPtr, outMax) {
    const pending = this.pendingFetches.get(fetchId);
    if (!pending)
      return -2;
    if (!pending.resolved)
      return -1;
    this.pendingFetches.delete(fetchId);
    if (pending.error || pending.contentLength === null)
      return -2;
    const buf = new ArrayBuffer(8);
    new DataView(buf).setBigUint64(0, BigInt(pending.contentLength), true);
    new Uint8Array((this._memory || this.w.memory).buffer, outPtr, 8).set(new Uint8Array(buf));
    return 8;
  }
  rangeStart(urlPtr, urlLen, offset, len) {
    const url = new TextDecoder().decode(new Uint8Array((this._memory || this.w.memory).buffer, urlPtr, urlLen));
    const fullUrl = this._resolveUrl(url);
    const id = this.nextFetchId++;
    this.pendingFetches.set(id, new RangeFetch(fullUrl, offset, len));
    return id;
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

// texture.client.ts
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
export {
  TextureClient
};
