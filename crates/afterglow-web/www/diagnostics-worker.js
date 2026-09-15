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
function encodeU8(n) {
  return new Uint8Array([n & 255]);
}
function decodeU8(bytes, off) {
  if (off >= bytes.length)
    throw new Error("postcard u8 truncated");
  return [bytes[off], off + 1];
}
function decodeU32(bytes, off) {
  return decodeVarint(bytes, off);
}
function encodeString(s) {
  const enc = new TextEncoder().encode(s);
  return concat(encodeVarint(enc.length), enc);
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

// crates/afterglow-web/web/src/workers/ring-service.ts
function installRingService(create) {
  const host = self;
  let state = "init";
  let memory;
  let requestBase = 0, responseBase = 0, capacity = 0;
  let handler;
  let wakePending = false;
  let wakeResolve = null;
  function fail(error) {
    state = "failed";
    host.postMessage({ type: "error", message: (error instanceof Error ? error.message : String(error)).slice(0, 512) });
  }
  host.onmessage = (event) => {
    const message = event.data;
    if (state === "init" && message?.type === "init") {
      try {
        const { sab, reqBase, respBase, bufSize } = message;
        if (!(sab instanceof SharedArrayBuffer) || !Number.isInteger(bufSize) || bufSize <= HEADER + 8 || !Number.isInteger(reqBase) || !Number.isInteger(respBase) || reqBase < 0 || respBase < 0 || reqBase % 4 !== 0 || respBase % 4 !== 0 || reqBase + bufSize > sab.byteLength || respBase + bufSize > sab.byteLength || Math.abs(reqBase - respBase) < bufSize) {
          throw new Error("Invalid RPC ring storage");
        }
        memory = sab;
        requestBase = reqBase;
        responseBase = respBase;
        capacity = bufSize - HEADER;
        if (Atomics.load(new Uint32Array(memory, requestBase, 1), 0) !== capacity || Atomics.load(new Uint32Array(memory, responseBase, 1), 0) !== capacity) {
          throw new Error("Invalid RPC ring capacity");
        }
        handler = create(message.workerInit);
        state = "ready";
        host.postMessage({ type: "ready" });
      } catch (error) {
        fail(error);
      }
      return;
    }
    if (state === "ready" && message?.type === "run") {
      state = "running";
      run().catch(fail);
      return;
    }
    if (state === "running" && message === "wake") {
      if (wakeResolve) {
        const resolve = wakeResolve;
        wakeResolve = null;
        resolve();
      } else
        wakePending = true;
    }
  };
  function wait() {
    if (wakePending) {
      wakePending = false;
      return Promise.resolve();
    }
    return new Promise((resolve) => {
      wakeResolve = resolve;
    });
  }
  async function run() {
    const requestWrite = new Int32Array(memory, requestBase + U32, 1);
    const requestRead = new Int32Array(memory, requestBase + 2 * U32, 1);
    const requestData = new Uint8Array(memory, requestBase + HEADER, capacity);
    const responseWrite = new Int32Array(memory, responseBase + U32, 1);
    const responseRead = new Int32Array(memory, responseBase + 2 * U32, 1);
    const responseData = new Uint8Array(memory, responseBase + HEADER, capacity);
    while (state === "running") {
      const write = Atomics.load(requestWrite, 0) >>> 0;
      const read = Atomics.load(requestRead, 0) >>> 0;
      const used = write - read >>> 0;
      if (!used) {
        await wait();
        continue;
      }
      if (used > capacity || used < U32)
        throw new Error("Corrupt RPC request ring");
      const offset = read % capacity;
      const length = rdU32(requestData, offset, capacity);
      const frame = U32 + length;
      if (length < U32 || frame > used || frame > capacity)
        throw new Error("Corrupt RPC request frame");
      const method = rdU32(requestData, offset + U32, capacity) >>> 0;
      const args = new Uint8Array(length - U32);
      xfer(requestData, offset + 2 * U32, capacity, args, args.byteLength, "rd");
      Atomics.store(requestRead, 0, read + frame >>> 0);
      let response;
      try {
        response = concat(encodeVarint(0), encodeBytes(await handler(method, args)));
      } catch (error) {
        const text = (error instanceof Error ? error.message : String(error)).slice(0, 512);
        response = concat(encodeVarint(1), encodeVarint(method), encodeString(text));
      }
      const responseFrame = U32 + response.byteLength;
      const responseIndex = Atomics.load(responseWrite, 0) >>> 0;
      const responseUsed = responseIndex - (Atomics.load(responseRead, 0) >>> 0) >>> 0;
      if (responseUsed > capacity || responseFrame > capacity - responseUsed)
        throw new Error("RPC response ring full");
      const responseOffset = responseIndex % capacity;
      wrU32(responseData, responseOffset, capacity, response.byteLength);
      xfer(responseData, responseOffset + U32, capacity, response, response.byteLength, "wr");
      Atomics.store(responseWrite, 0, responseIndex + responseFrame >>> 0);
      host.postMessage("wake");
    }
  }
}

// crates/afterglow-telemetry/web/src/websocket.ts
var MAX_MESSAGE_BYTES = 65536;
var EMPTY_STATUS = '{"connected":false}';
var EMPTY_BYTES = new Uint8Array(0);

class WebSocketCaptureClient {
  url;
  openSocket;
  socket = null;
  session = new Uint32Array(4);
  encoder = new TextEncoder;
  epoch = 0;
  ready = false;
  retryAt = 0;
  timeout;
  constructor(url = "ws://127.0.0.1:8086/", openSocket = (url2) => new WebSocket(url2)) {
    this.url = url;
    this.openSocket = openSocket;
    const endpoint = new URL(url);
    if (endpoint.protocol !== "ws:" || !["127.0.0.1", "[::1]", "localhost"].includes(endpoint.hostname) || endpoint.pathname !== "/" || endpoint.username || endpoint.password || endpoint.search || endpoint.hash) {
      throw new Error("Profiling needs a local WebSocket URL");
    }
    crypto.getRandomValues(this.session);
    if (this.session.every((value) => value === 0))
      throw new Error("Invalid profiling session");
  }
  async start() {
    if (!this.socket && performance.now() >= this.retryAt && this.epoch < 4294967295) {
      this.retryAt = performance.now() + 1000;
      try {
        const socket = this.openSocket(this.url);
        this.socket = socket;
        socket.binaryType = "arraybuffer";
        socket.onopen = this.onOpen;
        socket.onclose = this.onClose;
        socket.onerror = this.onClose;
        socket.onmessage = this.onClose;
        this.timeout = setTimeout(this.expire, 5000);
      } catch {
        this.disconnect();
      }
    }
    if (!this.ready)
      return EMPTY_STATUS;
    return JSON.stringify({
      connected: true,
      session: Array.from(this.session),
      epoch: this.epoch,
      nativeTick: String(Math.floor(performance.now() * 1e6)),
      maxFrameBytes: MAX_MESSAGE_BYTES,
      maxBatchRecords: 1024
    });
  }
  async ingest(epoch, opcode, payload) {
    if (!Number.isInteger(epoch) || ![1, 2, 3].includes(opcode) || payload.byteLength >= MAX_MESSAGE_BYTES) {
      throw new Error("Invalid profiling message");
    }
    if (!this.ready || epoch !== this.epoch)
      return 2;
    return this.send(opcode, payload) ? 0 : 2;
  }
  async finish() {
    if (this.ready)
      this.send(4, EMPTY_BYTES);
    this.disconnect();
    return EMPTY_STATUS;
  }
  onOpen = (event) => {
    if (event.target !== this.socket)
      return;
    clearTimeout(this.timeout);
    this.timeout = undefined;
    this.epoch++;
    const hello = this.encoder.encode(JSON.stringify({
      protocol: 3,
      session: Array.from(this.session),
      epoch: this.epoch,
      max_frame_bytes: MAX_MESSAGE_BYTES
    }));
    this.ready = this.send(0, hello);
  };
  onClose = (event) => {
    if (event.target === this.socket)
      this.disconnect();
  };
  expire = () => {
    this.disconnect();
  };
  send(opcode, payload) {
    const socket = this.socket;
    if (!socket || socket.readyState !== 1 || socket.bufferedAmount + payload.byteLength + 1 > MAX_MESSAGE_BYTES) {
      this.disconnect();
      return false;
    }
    const bytes = new Uint8Array(1 + payload.byteLength);
    bytes[0] = opcode;
    bytes.set(payload, 1);
    try {
      socket.send(bytes);
      return true;
    } catch {
      this.disconnect();
      return false;
    }
  }
  disconnect() {
    this.ready = false;
    clearTimeout(this.timeout);
    this.timeout = undefined;
    const socket = this.socket;
    this.socket = null;
    this.retryAt = performance.now() + 1000;
    if (socket) {
      socket.onopen = socket.onclose = socket.onerror = socket.onmessage = null;
      try {
        socket.close();
      } catch {}
    }
  }
}

// crates/afterglow-web/web/src/workers/diagnostics-worker.ts
installRingService((configuration) => {
  if (typeof configuration !== "object" || configuration === null || typeof configuration.url !== "string")
    throw new Error("Missing profiling server URL");
  const client = new WebSocketCaptureClient(configuration.url);
  return async (method, args) => {
    if (method === 0 && args.byteLength === 0)
      return encodeString(await client.start());
    if (method === 2 && args.byteLength === 0)
      return encodeString(await client.finish());
    if (method === 1) {
      const [epoch, a] = decodeU32(args, 0);
      const [opcode, b] = decodeU8(args, a);
      const [payload, end] = decodeBytes(args, b);
      if (end !== args.byteLength)
        throw new Error("Trailing profiling RPC data");
      return encodeU8(await client.ingest(epoch, opcode, payload));
    }
    throw new Error("Invalid profiling RPC method");
  };
});
