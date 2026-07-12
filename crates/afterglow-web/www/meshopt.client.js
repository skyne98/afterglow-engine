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

// meshopt.client.ts
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
export {
  MeshoptClient
};
