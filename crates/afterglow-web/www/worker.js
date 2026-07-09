// Generic Web Worker for afterglow-rpc workers.
//
// The worker has its OWN wasm memory (not shared) — allocations inside serve
// are safe. It reads requests from and writes responses to the main thread's
// SharedArrayBuffer ring buffer via JS (DataView + Atomics).
//
// Ring buffer layout: [cap:u32][write_idx:AtomicU32][read_idx:AtomicU32][data...]
// Frames: [len:u32][payload]

let sab = null;          // SharedArrayBuffer (the main thread's wasm memory)
let reqBase = 0;         // request ring buffer offset in the SAB
let respBase = 0;        // response ring buffer offset in the SAB
let bufSize = 0;
let wasm = null;         // the worker's own wasm instance (own memory)
let wasmMem = null;      // the worker's own WebAssembly.Memory

self.onmessage = async (e) => {
  if (e.data.type === 'init') {
    sab = e.data.sab;
    reqBase = e.data.reqBase;
    respBase = e.data.respBase;
    bufSize = e.data.bufSize;

    // Fetch + compile the wasm module (same module, but we give it its OWN memory)
    const bytes = await fetch(e.data.wasmUrl).then(r => r.arrayBuffer());
    const module = await WebAssembly.compile(bytes);

    // Own memory — shared:true to satisfy the module's --shared-memory flag,
    // but this is the worker's PRIVATE memory (not shared with the main thread).
    // Allocations inside serve are isolated — no allocator conflict.
    wasmMem = new WebAssembly.Memory({ shared: true, initial: 256, maximum: 1024 });
    const instance = await WebAssembly.instantiate(module, { env: { memory: wasmMem } });
    wasm = instance.exports;

    // Initialize the worker's server impl (calls wasm_init_worker internally)
    if (wasm.wasm_init) wasm.wasm_init();

    self.postMessage({ type: 'ready' });
    return;
  }

  if (e.data.type === 'call_init') {
    // Call wasm_init_worker if the service exports it
    // (the user must have called it during init, or we call it here)
    // For now, the user sets up the impl in their wasm module's start function
    self.postMessage({ type: 'init_done' });
    return;
  }

  if (e.data.type === 'run') {
    // Main worker loop: read requests from SAB, call wasm_serve_frame, write responses
    const dataLen = bufSize - 12;
    const reqCap = new Uint32Array(sab, reqBase, 1)[0];
    const reqW = new Int32Array(sab, reqBase + 4, 1);
    const reqR = new Int32Array(sab, reqBase + 8, 1);
    const reqData = new Uint8Array(sab, reqBase + 12, dataLen);

    const respW = new Int32Array(sab, respBase + 4, 1);
    const respR = new Int32Array(sab, respBase + 8, 1);
    const respData = new Uint8Array(sab, respBase + 12, dataLen);

    // Scratch buffers in the worker's OWN wasm memory (for serve args + response)
    const scratchBase = 1024;  // offset in worker's wasm memory
    const scratchSize = 4 * 1024 * 1024;  // 4 MiB
    const responseBase = scratchBase + scratchSize;
    const responseSize = 4 * 1024 * 1024;

    let processed = 0;

    while (true) {
      // Check for request
      const w = Atomics.load(reqW, 0);
      const r = Atomics.load(reqR, 0);
      if (w === r) {
        // Block up to 1ms waiting for a request (main thread can't
        // Atomics.notify from Rust, so we use a short timeout).
        Atomics.wait(reqW, 0, w, 1);
        continue;
      }

      // Read frame: [len:u32][payload]
      const off = r % dataLen;
      const dv = new DataView(sab, reqBase + 12, dataLen);
      const payloadLen = dv.getUint32(off, true);
      const frameLen = 4 + payloadLen;

      // Read method (first 4 bytes of payload) + args
      let methodOff = (off + 4) % dataLen;
      const method = dv.getUint32(methodOff, true);
      const argsLen = payloadLen - 4; // subtract method u32

      // Copy args to worker's own wasm memory
      const wasmView = new Uint8Array(wasmMem.buffer, scratchBase, argsLen);
      const first = Math.min(argsLen, dataLen - ((methodOff + 4) % dataLen));
      const argsStart = (methodOff + 4) % dataLen;
      wasmView.set(reqData.subarray(argsStart, argsStart + first));
      if (first < argsLen) {
        wasmView.set(reqData.subarray(0, argsLen - first), first);
      }

      // Advance read index
      Atomics.store(reqR, 0, (r + frameLen) >>> 0);

      // Call serve in the worker's own wasm instance
      const respLen = wasm.wasm_serve_frame(method, scratchBase, argsLen, responseBase, responseSize);

      if (respLen > 0) {
        // Write response to SAB
        const respView = new Uint8Array(wasmMem.buffer, responseBase, respLen);
        // Write frame: [len:u32][payload]
        const respFrameLen = 4 + respLen;

        // Check if response buffer has space
        const rwVal = Atomics.load(respW, 0);
        const rrVal = Atomics.load(respR, 0);
        const used = (rwVal - rrVal) >>> 0;
        if (respFrameLen > reqCap - used) {
          // Response buffer full — retry
          continue;
        }

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

      processed++;
      if (processed % 1000 === 0) {
        await new Promise(resolve => setTimeout(resolve, 0));
      }
    }
  }
};
