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

// crates/afterglow-web/web/src/workers/worker.ts
var state = "init";
var sab = null;
var reqBase = 0;
var respBase = 0;
var bufSize = 0;
var wasm = null;
var wasmMem = null;
var inputPtr = 0;
var inputSize = 0;
var outputPtr = 0;
var outputSize = 0;
var wakePending = 0;
var wakeResolve = null;
self.onmessage = async (e) => {
  const m = e.data;
  if (state === "init" && m && m.type === "init") {
    await initWorker(m);
    return;
  }
  if (state === "ready" && m && m.type === "run") {
    state = "running";
    runLoop();
    return;
  }
  if (state === "running" && m === "wake") {
    if (wakeResolve) {
      const r = wakeResolve;
      wakeResolve = null;
      r();
    } else
      wakePending = 1;
  }
};
async function initWorker(m) {
  try {
    sab = m.sab;
    reqBase = m.reqBase;
    respBase = m.respBase;
    bufSize = m.bufSize;
    const bytes = await fetch(m.wasmUrl).then((r) => r.arrayBuffer());
    wasmMem = new WebAssembly.Memory({ shared: true, initial: 256, maximum: 1024 });
    wasm = (await WebAssembly.instantiate(await WebAssembly.compile(bytes), { env: {
      memory: wasmMem,
      notify_worker: () => {},
      performance_now: () => performance.now(),
      ag_fetch_start: () => 0,
      ag_fetch_poll: () => -2,
      ag_fetch_head_start: () => 0,
      ag_fetch_head_poll: () => -2,
      ag_fetch_range_start: () => 0
    } })).exports;
    if (wasm.afterglow_wasm_init)
      wasm.afterglow_wasm_init();
    inputPtr = wasm.afterglow_wasm_input_ptr();
    inputSize = wasm.afterglow_wasm_input_size();
    outputPtr = wasm.afterglow_wasm_output_ptr();
    outputSize = wasm.afterglow_wasm_output_size();
    state = "ready";
    self.postMessage({ type: "ready" });
  } catch (e) {
    self.postMessage({ type: "error", message: String(e && e.message || e) });
  }
}
function waitForWake() {
  if (wakePending) {
    wakePending = 0;
    return Promise.resolve();
  }
  return new Promise((r) => {
    wakeResolve = r;
  });
}
async function runLoop() {
  const reqCap = Atomics.load(new Uint32Array(sab, reqBase, 1), 0) >>> 0;
  const respCap = Atomics.load(new Uint32Array(sab, respBase, 1), 0) >>> 0;
  if (reqCap === 0 || reqCap !== bufSize - HEADER || respCap === 0 || respCap !== bufSize - HEADER) {
    self.postMessage({ type: "error", message: "bad ring capacity" });
    return;
  }
  const reqW = new Int32Array(sab, reqBase + U32, 1);
  const reqR = new Int32Array(sab, reqBase + 2 * U32, 1);
  const reqData = new Uint8Array(sab, reqBase + HEADER, reqCap);
  const respW = new Int32Array(sab, respBase + U32, 1);
  const respR = new Int32Array(sab, respBase + 2 * U32, 1);
  const respData = new Uint8Array(sab, respBase + HEADER, respCap);
  for (;; ) {
    const w = Atomics.load(reqW, 0) >>> 0;
    const r = Atomics.load(reqR, 0) >>> 0;
    const used = w - r >>> 0;
    if (used === 0) {
      await waitForWake();
      continue;
    }
    if (used > reqCap || used < U32) {
      Atomics.store(reqR, 0, w >>> 0);
      self.postMessage({ type: "error", message: "corrupt request ring" });
      continue;
    }
    const off = r % reqCap;
    const payloadLen = rdU32(reqData, off, reqCap);
    const frameLen = U32 + payloadLen;
    if (payloadLen < U32 || frameLen > used || frameLen > reqCap) {
      Atomics.store(reqR, 0, w >>> 0);
      self.postMessage({ type: "error", message: "corrupt request frame" });
      continue;
    }
    const method = rdU32(reqData, off + U32, reqCap) >>> 0;
    const argsLen = payloadLen - U32;
    if (argsLen > inputSize) {
      Atomics.store(reqR, 0, r + frameLen >>> 0);
      self.postMessage({ type: "error", message: "request too large for scratch" });
      continue;
    }
    xfer(reqData, off + 2 * U32, reqCap, new Uint8Array(wasmMem.buffer, inputPtr, argsLen), argsLen, "rd");
    Atomics.store(reqR, 0, r + frameLen >>> 0);
    let responsePtr = outputPtr;
    let respLen;
    if (wasm.afterglow_wasm_serve_async) {
      const started = wasm.afterglow_wasm_serve_async(method, inputPtr, argsLen, 1n);
      if (started < 0) {
        self.postMessage({ type: "error", message: "serve_async failed" });
        continue;
      }
      let completionLen = -1;
      while (completionLen < 0) {
        wasm.afterglow_wasm_tick();
        completionLen = wasm.afterglow_wasm_drain_completion(outputPtr, Math.min(outputSize, respCap - U32 + 8));
        if (completionLen < 0)
          await new Promise((resolve) => setTimeout(resolve, 0));
      }
      if (completionLen < 8) {
        self.postMessage({ type: "error", message: "short async completion" });
        continue;
      }
      responsePtr += 8;
      respLen = completionLen - 8;
    } else {
      respLen = wasm.afterglow_wasm_serve_frame(method, inputPtr, argsLen, outputPtr, Math.min(outputSize, respCap - U32));
    }
    if (respLen < 0 || respLen > outputSize) {
      self.postMessage({ type: "error", message: "worker service failed" });
      continue;
    }
    const rw = Atomics.load(respW, 0) >>> 0;
    const rr = Atomics.load(respR, 0) >>> 0;
    const respUsed = rw - rr >>> 0;
    const respFrameLen = U32 + respLen;
    if (respFrameLen > respCap || respFrameLen > respCap - respUsed) {
      self.postMessage({ type: "error", message: "response ring full" });
      continue;
    }
    const roff = rw % respCap;
    wrU32(respData, roff, respCap, respLen >>> 0);
    xfer(respData, roff + U32, respCap, new Uint8Array(wasmMem.buffer, responsePtr, respLen), respLen, "wr");
    Atomics.store(respW, 0, rw + respFrameLen >>> 0);
    self.postMessage("wake");
  }
}
