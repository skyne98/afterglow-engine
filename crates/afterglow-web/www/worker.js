// Generic Web Worker for afterglow-rpc workers.
//
// The worker has its OWN wasm memory (not shared) — allocations inside serve
// are safe. It reads requests from and writes responses to the main thread's
// SharedArrayBuffer ring buffer via JS (DataView + Atomics).
//
// Wake-up strategy (hybrid):
//   1. Tight non-blocking spin (Atomics.wait timeout=0) for 10k iterations —
//      catches requests within microseconds during active communication.
//   2. If no request after spinning, await postMessage (instant wake-up from
//      the main thread's notify_worker import) with a 1ms timeout fallback.
//
// Ring buffer layout: [cap:u32][write_idx:AtomicU32][read_idx:AtomicU32][data...]
// Frames: [len:u32][payload]

let sab = null;
let reqBase = 0;
let respBase = 0;
let bufSize = 0;
let wasm = null;
let wasmMem = null;
let wakeResolve = null;

self.onmessage = async (e) => {
  if (e.data.type === 'init') {
    sab = e.data.sab;
    reqBase = e.data.reqBase;
    respBase = e.data.respBase;
    bufSize = e.data.bufSize;

    const bytes = await fetch(e.data.wasmUrl).then(r => r.arrayBuffer());
    const module = await WebAssembly.compile(bytes);

    wasmMem = new WebAssembly.Memory({ shared: true, initial: 256, maximum: 1024 });
    const instance = await WebAssembly.instantiate(module, {
      env: { memory: wasmMem, notify_worker: () => {} }
    });
    wasm = instance.exports;
    if (wasm.wasm_init) wasm.wasm_init();

    self.postMessage({ type: 'ready' });
    return;
  }

  if (e.data.type === 'run') {
    const dataLen = bufSize - 12;
    const reqCap = new Uint32Array(sab, reqBase, 1)[0];
    const reqW = new Int32Array(sab, reqBase + 4, 1);
    const reqR = new Int32Array(sab, reqBase + 8, 1);
    const reqData = new Uint8Array(sab, reqBase + 12, dataLen);

    const respW = new Int32Array(sab, respBase + 4, 1);
    const respR = new Int32Array(sab, respBase + 8, 1);
    const respData = new Uint8Array(sab, respBase + 12, dataLen);

    const scratchBase = 1024;
    const responseBase = scratchBase + 4 * 1024 * 1024;
    const responseSize = 4 * 1024 * 1024;

    // Wake-up handler: resolves the sleep promise when postMessage('wake') arrives
    self.onmessage = (e) => {
      if (e.data === 'wake' && wakeResolve) {
        const r = wakeResolve;
        wakeResolve = null;
        r();
      }
    };

    const SPIN_LIMIT = 10000;

    while (true) {
      const w = Atomics.load(reqW, 0);
      const r = Atomics.load(reqR, 0);

      if (w === r) {
        // Phase 1: tight non-blocking spin — catches requests within µs
        // when the main thread is actively sending.
        let spins = 0;
        while (spins < SPIN_LIMIT) {
          if (Atomics.load(reqW, 0) !== w) break;
          Atomics.wait(reqW, 0, w, 0); // non-blocking
          spins++;
        }
        if (spins < SPIN_LIMIT) continue; // got a request during spin

        // Phase 2: idle — await postMessage (instant) or 1ms timeout (fallback)
        await new Promise(resolve => {
          wakeResolve = resolve;
          setTimeout(resolve, 1);
        });
        continue;
      }

      // --- Process request ---

      const off = r % dataLen;
      const dv = new DataView(sab, reqBase + 12, dataLen);
      const payloadLen = dv.getUint32(off, true);
      const frameLen = 4 + payloadLen;

      let methodOff = (off + 4) % dataLen;
      const method = dv.getUint32(methodOff, true);
      const argsLen = payloadLen - 4;

      // Copy args to worker's own wasm memory
      const wasmView = new Uint8Array(wasmMem.buffer, scratchBase, argsLen);
      const argsStart = (methodOff + 4) % dataLen;
      const first = Math.min(argsLen, dataLen - argsStart);
      wasmView.set(reqData.subarray(argsStart, argsStart + first));
      if (first < argsLen) {
        wasmView.set(reqData.subarray(0, argsLen - first), first);
      }

      Atomics.store(reqR, 0, (r + frameLen) >>> 0);

      // Call serve in the worker's own wasm instance
      const respLen = wasm.wasm_serve_frame(method, scratchBase, argsLen, responseBase, responseSize);

      if (respLen > 0) {
        const respView = new Uint8Array(wasmMem.buffer, responseBase, respLen);
        const respFrameLen = 4 + respLen;

        const rwVal = Atomics.load(respW, 0);
        const rrVal = Atomics.load(respR, 0);
        const used = (rwVal - rrVal) >>> 0;
        if (respFrameLen > reqCap - used) continue;

        const respOff = rwVal % dataLen;
        const respDv = new DataView(sab, respBase + 12, dataLen);
        respDv.setUint32(respOff, respLen, true);

        let respPayloadOff = (respOff + 4) % dataLen;
        const respFirst = Math.min(respLen, dataLen - respPayloadOff);
        respData.set(respView.subarray(0, respFirst), respPayloadOff);
        if (respFirst < respLen) {
          respData.set(respView.subarray(respFirst), 0);
        }

        Atomics.store(respW, 0, (rwVal + respFrameLen) >>> 0);
      }
    }
  }
};
