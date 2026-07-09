// Benchmark worker: pure JS ring buffer access via SharedArrayBuffer.
//
// No wasm instance in the worker — avoids stack/heap corruption between
// multiple wasm instances sharing the same memory.
//
// Ring buffer layout: [cap:u32][write_idx:AtomicU32][read_idx:AtomicU32][data...]
// Frames: [len:u32][payload]

let mem = null;       // SharedArrayBuffer
let reqBase = 0;      // request buffer offset (main→worker)
let respBase = 0;     // response buffer offset (worker→main)
let bufSize = 0;
let dataLen = 0;      // ring buffer data area length

function setupRing(base) {
  return {
    capView: new Uint32Array(mem, base, 1),
    wIdx: new Uint32Array(mem, base + 4, 1),
    rIdx: new Uint32Array(mem, base + 8, 1),
    data: new Uint8Array(mem, base + 12, dataLen - 12),
    dataBase: base + 12,
  };
}

function rbWrite(rb, payload) {
  const w = Atomics.load(rb.wIdx, 0);
  const r = Atomics.load(rb.rIdx, 0);
  const used = (w - r) >>> 0;
  const avail = rb.capView[0] - used;
  const frameLen = 4 + payload.length;
  if (frameLen > avail) return false;

  const off = w % rb.data.length;
  const dv = new DataView(mem, rb.dataBase, rb.data.length);

  // Write length prefix
  dv.setUint32(off, payload.length, true);
  // Write payload (with wrap)
  let pOff = (off + 4) % rb.data.length;
  const first = Math.min(payload.length, rb.data.length - pOff);
  rb.data.set(payload.subarray(0, first), pOff);
  if (first < payload.length) {
    rb.data.set(payload.subarray(first), 0);
  }

  Atomics.store(rb.wIdx, 0, (w + frameLen) >>> 0);
  return true;
}

function rbRead(rb, outBuf) {
  const w = Atomics.load(rb.wIdx, 0);
  const r = Atomics.load(rb.rIdx, 0);
  if (w === r) return -1;

  const off = r % rb.data.length;
  const dv = new DataView(mem, rb.dataBase, rb.data.length);
  const payloadLen = dv.getUint32(off, true);
  const frameLen = 4 + payloadLen;

  let pOff = (off + 4) % rb.data.length;
  const n = Math.min(payloadLen, outBuf.length);
  const first = Math.min(n, rb.data.length - pOff);
  outBuf.set(rb.data.subarray(pOff, pOff + first), 0);
  if (first < n) {
    outBuf.set(rb.data.subarray(0, n - first), first);
  }

  Atomics.store(rb.rIdx, 0, (r + frameLen) >>> 0);
  return n;
}

function rbHasData(rb) {
  return Atomics.load(rb.wIdx, 0) !== Atomics.load(rb.rIdx, 0);
}

self.onmessage = (e) => {
  if (e.data.type === 'init') {
    // We receive the shared memory + offsets. No wasm instance needed.
    mem = e.data.sab;  // SharedArrayBuffer
    reqBase = e.data.reqBase;
    respBase = e.data.respBase;
    bufSize = e.data.bufSize;
    dataLen = bufSize;
    self.postMessage({ type: 'ready' });
    return;
  }

  if (e.data.type === 'benchmark') {
    const { mode, size, count } = e.data;
    const req = setupRing(reqBase);
    const resp = setupRing(respBase);
    const payload = new Uint8Array(size).fill(0xBB);
    const readBuf = new Uint8Array(Math.max(size + 64, 1024));
    let ok = 0;

    if (mode === 'drain') {
      for (let i = 0; i < count; i++) {
        while (!rbHasData(req)) {}
        rbRead(req, readBuf);
        ok++;
      }
    } else if (mode === 'fill') {
      for (let i = 0; i < count; i++) {
        while (!rbWrite(resp, payload)) {}
        ok++;
      }
    } else if (mode === 'echo') {
      for (let i = 0; i < count; i++) {
        while (!rbHasData(req)) {}
        rbRead(req, readBuf);
        while (!rbWrite(resp, readBuf.subarray(0, size))) {}
        ok++;
      }
    }

    self.postMessage({ type: 'done', mode, size, ok });
    return;
  }

  if (e.data.type === 'stop') {
    self.close();
  }
};
