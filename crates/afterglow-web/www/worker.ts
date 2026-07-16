// afterglow-web Web Worker. Has its OWN wasm memory (shared:true, separate
// from the main thread's SAB). Reads requests from / writes responses to the
// main SAB ring buffers via Atomics + ring buffer wrap-safe helpers (imported
// from ring-buf.js). For each request it copies args into the worker wasm
// input scratch (pointer/size from exports, never hard-coded), calls
// `afterglow_wasm_serve_frame`, and copies the response envelope into the SAB
// response ring.
//
// Ring layout: [cap:u32][write_idx:u32][read_idx:u32][data...]. A frame in
// data: [len:u32 LE][payload], payload = [method:u32 LE][args]. All ring
// accesses use the wrap-safe helpers from ring-buf.js. Wake state is lossless
// with no idle CPU spin: the worker awaits a postMessage('wake') from the main
// thread; a wake that arrives while not awaiting is recorded in `wakePending`
// and consumed next iteration. A response wake is posted to main only AFTER
// the response ring index is published, and carries no payload (the ring is
// the transport).

import { HEADER, U32, rdU32, wrU32, xfer } from './ring-buf.ts';

let state = 'init';         // 'init' -> 'ready' -> 'running'
let sab = null;             // main thread's SharedArrayBuffer (its wasm memory)
let reqBase = 0, respBase = 0, bufSize = 0;
let wasm = null;            // worker wasm exports
let wasmMem = null;         // worker's own WebAssembly.Memory
let inputPtr = 0, inputSize = 0, outputPtr = 0, outputSize = 0;
let wakePending = 0, wakeResolve = null;

self.onmessage = async (e) => {
  const m = e.data;
  if (state === 'init' && m && m.type === 'init') { await initWorker(m); return; }
  if (state === 'ready' && m && m.type === 'run') { state = 'running'; runLoop(); return; }
  if (state === 'running' && m === 'wake') {
    if (wakeResolve) { const r = wakeResolve; wakeResolve = null; r(); }
    else wakePending = 1;
  }
};

async function initWorker(m) {
  try {
    sab = m.sab; reqBase = m.reqBase; respBase = m.respBase; bufSize = m.bufSize;
    const bytes = await fetch(m.wasmUrl).then(r => r.arrayBuffer());
    wasmMem = new WebAssembly.Memory({ shared: true, initial: 256, maximum: 1024 });
    wasm = (await WebAssembly.instantiate(
      await WebAssembly.compile(bytes),
      { env: {
        memory: wasmMem,
        notify_worker: () => {},
        performance_now: () => performance.now(),
        // Texture/mesh workers do not fetch. These satisfy the shared async
        // worker ABI and fail deterministically if a wrong service uses them.
        ag_fetch_start: () => 0,
        ag_fetch_poll: () => -2,
        ag_fetch_head_start: () => 0,
        ag_fetch_head_poll: () => -2,
        ag_fetch_range_start: () => 0,
      } },
    )).exports;
    if (wasm.afterglow_wasm_init) wasm.afterglow_wasm_init();
    inputPtr = wasm.afterglow_wasm_input_ptr();
    inputSize = wasm.afterglow_wasm_input_size();
    outputPtr = wasm.afterglow_wasm_output_ptr();
    outputSize = wasm.afterglow_wasm_output_size();
    state = 'ready';
    self.postMessage({ type: 'ready' });
  } catch (e) {
    self.postMessage({ type: 'error', message: String((e && e.message) || e) });
  }
}

// Lossless wake: consume a pending wake, else await the next postMessage('wake').
function waitForWake() {
  if (wakePending) { wakePending = 0; return Promise.resolve(); }
  return new Promise(r => { wakeResolve = r; });
}

async function runLoop() {
  // Validate request and response capacities separately against the buffer.
  const reqCap = Atomics.load(new Uint32Array(sab, reqBase, 1), 0) >>> 0;
  const respCap = Atomics.load(new Uint32Array(sab, respBase, 1), 0) >>> 0;
  if (reqCap === 0 || reqCap !== bufSize - HEADER
      || respCap === 0 || respCap !== bufSize - HEADER) {
    self.postMessage({ type: 'error', message: 'bad ring capacity' });
    return;
  }
  const reqW = new Int32Array(sab, reqBase + U32, 1);
  const reqR = new Int32Array(sab, reqBase + 2 * U32, 1);
  const reqData = new Uint8Array(sab, reqBase + HEADER, reqCap);
  const respW = new Int32Array(sab, respBase + U32, 1);
  const respR = new Int32Array(sab, respBase + 2 * U32, 1);
  const respData = new Uint8Array(sab, respBase + HEADER, respCap);

  for (;;) {
    const w = Atomics.load(reqW, 0) >>> 0;
    const r = Atomics.load(reqR, 0) >>> 0;
    const used = (w - r) >>> 0;
    if (used === 0) { await waitForWake(); continue; }
    // used must hold at least a u32 header; resync (drain) + report on inconsistency.
    if (used > reqCap || used < U32) { Atomics.store(reqR, 0, w >>> 0); self.postMessage({ type: 'error', message: 'corrupt request ring' }); continue; }
    const off = r % reqCap;
    const payloadLen = rdU32(reqData, off, reqCap);
    const frameLen = U32 + payloadLen;
    if (payloadLen < U32 || frameLen > used || frameLen > reqCap) {
      Atomics.store(reqR, 0, w >>> 0); self.postMessage({ type: 'error', message: 'corrupt request frame' }); continue;
    }
    const method = rdU32(reqData, off + U32, reqCap) >>> 0;
    const argsLen = payloadLen - U32;
    if (argsLen > inputSize) {
      Atomics.store(reqR, 0, (r + frameLen) >>> 0); self.postMessage({ type: 'error', message: 'request too large for scratch' }); continue;
    }
    // Copy args into the worker's own input scratch. Fresh view each iteration:
    // serve_frame may have grown the worker memory on a prior turn (detaching
    // the old buffer). Wrap-safe copy handles a frame straddling the data end.
    xfer(reqData, off + 2 * U32, reqCap,
         new Uint8Array(wasmMem.buffer, inputPtr, argsLen), argsLen, 'rd');
    Atomics.store(reqR, 0, (r + frameLen) >>> 0); // consume request

    let responsePtr = outputPtr;
    let respLen;
    if (wasm.afterglow_wasm_serve_async) {
      // CPU-heavy async RPC methods still execute synchronously when polled,
      // but polling here keeps that work off the page/main thread.
      const started = wasm.afterglow_wasm_serve_async(method, inputPtr, argsLen, 1n);
      if (started < 0) { self.postMessage({ type: 'error', message: 'serve_async failed' }); continue; }
      let completionLen = -1;
      while (completionLen < 0) {
        wasm.afterglow_wasm_tick();
        completionLen = wasm.afterglow_wasm_drain_completion(outputPtr, Math.min(outputSize, respCap - U32 + 8));
        if (completionLen < 0) await new Promise(resolve => setTimeout(resolve, 0));
      }
      if (completionLen < 8) { self.postMessage({ type: 'error', message: 'short async completion' }); continue; }
      responsePtr += 8; // strip task_id; the ring protocol carries Response
      respLen = completionLen - 8;
    } else {
      respLen = wasm.afterglow_wasm_serve_frame(method, inputPtr, argsLen, outputPtr, Math.min(outputSize, respCap - U32));
    }
    if (respLen < 0 || respLen > outputSize) {
      self.postMessage({ type: 'error', message: 'worker service failed' });
      continue;
    }
    // Publish the response frame [len:u32][envelope] BEFORE posting the wake.
    const rw = Atomics.load(respW, 0) >>> 0;
    const rr = Atomics.load(respR, 0) >>> 0;
    const respUsed = (rw - rr) >>> 0;
    const respFrameLen = U32 + respLen;
    if (respFrameLen > respCap || respFrameLen > respCap - respUsed) {
      self.postMessage({ type: 'error', message: 'response ring full' });
      continue;
    }
    const roff = rw % respCap;
    wrU32(respData, roff, respCap, respLen >>> 0);
    xfer(respData, roff + U32, respCap,
         new Uint8Array(wasmMem.buffer, responsePtr, respLen), respLen, 'wr');
    Atomics.store(respW, 0, (rw + respFrameLen) >>> 0);
    self.postMessage('wake'); // response wake (no payload)
  }
}
