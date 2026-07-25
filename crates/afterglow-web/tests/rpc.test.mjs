// Node test runner (no npm deps) for the afterglow-web main-thread RPC client.
// Covers the lifecycle hardening: fatal transport failures are latched
// permanently, late response wakes never drain as a later call's result, idle
// worker errors are retained, terminate() is idempotent cleanup, and
// Rpc.create never leaks a Worker on setup/init failure.
//
// Run: node --test crates/afterglow-web/tests/rpc.test.mjs
//
// The shipped rpc.js is an ESM module with no nearby package.json, so a bare
// `import('../www/rpc.js')` would be treated as CommonJS and fail on `export`.
// We load the actual file text and import it via a `data:text/javascript` URL
// (MIME, not package.json, decides ESM there) — no test code is shipped under
// www/ and no dependency is added.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const rpcSrc = readFileSync(fileURLToPath(new URL('../www/rpc.js', import.meta.url)), 'utf8');
const rpcModule = await import('data:text/javascript;base64,' + Buffer.from(rpcSrc, 'utf8').toString('base64'));
const { Rpc } = rpcModule;
const codecSrc = readFileSync(fileURLToPath(new URL('../www/codec.js', import.meta.url)), 'utf8');
const { decodeVarint, decodeF32Vec, unwrapResponse } = await import('data:text/javascript;base64,' + Buffer.from(codecSrc, 'utf8').toString('base64'));
const ringSrc = readFileSync(fileURLToPath(new URL('../www/ring-buf.js', import.meta.url)), 'utf8');
const { rdU32, wrU32, xfer } = await import('data:text/javascript;base64,' + Buffer.from(ringSrc, 'utf8').toString('base64'));

// --- codec and ring helpers ------------------------------------------------

test('postcard varints reject truncation and overflow', () => {
  assert.throws(() => decodeVarint(new Uint8Array([0x80]), 0), /truncated/);
  assert.throws(() => decodeVarint(new Uint8Array([0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]), 0), /overflows/);
  assert.deepEqual(decodeVarint(new Uint8Array([0xac, 0x02]), 0), [300, 2]);
});

test('response and f32-vector decoders reject truncated payloads', () => {
  assert.throws(() => unwrapResponse(new Uint8Array([0, 3, 1])), /truncated/);
  assert.throws(() => unwrapResponse(new Uint8Array([1, 0, 4, 65])), /truncated/);
  assert.throws(() => decodeF32Vec(new Uint8Array([2, 0, 0, 0, 0]), 0), /truncated/);
});

test('ring helpers preserve wrapped u32 and payload transfers', () => {
  const ring = new Uint8Array(8);
  wrU32(ring, 6, ring.length, 0x89abcdef);
  assert.equal(rdU32(ring, 6, ring.length), 0x89abcdef);

  const input = new Uint8Array([1, 2, 3, 4, 5]);
  xfer(ring, 6, ring.length, input, input.length, 'wr');
  const output = new Uint8Array(input.length);
  xfer(ring, 6, ring.length, output, output.length, 'rd');
  assert.deepEqual([...output], [...input]);
});

// --- fakes ----------------------------------------------------------------

class FakeWorker {
  constructor() {
    this.terminated = 0;
    this.posts = [];
    this._onmessage = null;
    this._onerror = null;
  }
  set onmessage(fn) { this._onmessage = fn; }
  get onmessage() { return this._onmessage; }
  set onerror(fn) { this._onerror = fn; }
  get onerror() { return this._onerror; }
  postMessage(msg) { this.posts.push(msg); }
  terminate() { this.terminated++; }
  // Deliver a worker->main message event (the main thread reads `e.data`).
  send(data) { if (this._onmessage) this._onmessage({ data }); }
  // Deliver a worker `onerror` event.
  error(e) { if (this._onerror) this._onerror(e); }
}

function makeWasm({ scratchSize = 1024, writeFrame = () => 0, readResponse = null } = {}) {
  const calls = { writeFrame: 0, readResponse: 0 };
  return {
    _calls: calls,
    get_scratch_ptr: () => 0,
    get_scratch_size: () => scratchSize,
    write_frame: (ptr, len) => { calls.writeFrame++; return writeFrame(ptr, len); },
    read_response: (ptr, max) => { calls.readResponse++; return readResponse ? readResponse(ptr, max) : -1; },
  };
}

// Build an Rpc directly against fakes (bypasses Rpc.create's fetch/wasm/Worker).
// `ready` sends the worker 'ready' message so the init timer is cleared and the
// client is in the normal post-init state.
function makeRig({ scratchSize = 1024, timeoutMs = 30, ready = true, writeFrame, readResponse } = {}) {
  const memory = { buffer: new ArrayBuffer(scratchSize) };
  const wasm = makeWasm({ scratchSize, writeFrame, readResponse });
  const worker = new FakeWorker();
  const rpc = new Rpc(wasm, memory, worker, { timeoutMs });
  if (ready) worker.send({ type: 'ready' });
  return { rpc, worker, wasm, memory };
}

const EMPTY = new Uint8Array(0);

// --- tests ----------------------------------------------------------------

test('happy path: a response wake resolves the call with the payload', async () => {
  const { rpc, worker, wasm, memory } = makeRig({ timeoutMs: 1000 });
  const payload = new Uint8Array([0xab, 0xcd]);
  // postcard Response::Ok(Vec<u8>) = [0][varint len][bytes]
  const ok = new Uint8Array([0, payload.length, ...payload]);
  let reads = 0;
  wasm.read_response = (ptr) => { reads++; new Uint8Array(memory.buffer, ptr, ok.length).set(ok); return ok.length; };

  const p = rpc.call(0, EMPTY);
  worker.send('wake'); // response wake (payload-free)
  const result = await p;
  assert.deepEqual([...result], [...payload]);
  assert.equal(reads, 1);
});

test('response timeout poisons Rpc; later calls fail immediately and stale wakes are not drained', async () => {
  const { rpc, worker, wasm } = makeRig({ timeoutMs: 30 });
  // write_frame succeeds; no response wake is ever sent -> the call must time out.
  const p = rpc.call(0, EMPTY);
  await assert.rejects(p, /RPC timeout/);
  assert.equal(wasm._calls.writeFrame, 1);   // exactly one write (the first call)
  assert.equal(wasm._calls.readResponse, 0); // nothing was ever read

  // Subsequent calls fail immediately, without touching the rings.
  await assert.rejects(rpc.call(0, EMPTY), /RPC timeout/);
  assert.equal(wasm._calls.writeFrame, 1);  // no additional write_frame
  assert.equal(wasm._calls.readResponse, 0);

  // A late response wake (stale response for the timed-out call) and a late
  // ready must not drain the ring or resurrect any state.
  worker.send('wake');
  worker.send({ type: 'ready' });
  assert.equal(wasm._calls.readResponse, 0); // still never read
  await assert.rejects(rpc.call(0, EMPTY), /RPC timeout/); // still poisoned
  assert.equal(wasm._calls.writeFrame, 1);
});

test('idle worker {type:error} (no pending call) is retained and fails the next call', async () => {
  const { rpc, worker, wasm } = makeRig({ timeoutMs: 30 });
  // No pending call. The worker reports a runtime error out of the blue.
  worker.send({ type: 'error', message: 'runtime boom' });
  assert.ok(rpc._fatal, 'fatal error latched');
  assert.match(rpc._fatal.message, /runtime boom/);

  // Next call fails immediately, without writing to the ring.
  await assert.rejects(rpc.call(0, EMPTY), /runtime boom/);
  assert.equal(wasm._calls.writeFrame, 0);
  assert.equal(wasm._calls.readResponse, 0);
});

test('idle worker.onerror (no pending call) is retained and fails the next call', async () => {
  const { rpc, worker, wasm } = makeRig({ timeoutMs: 30 });
  worker.error(new Error('uncaught boom'));
  assert.ok(rpc._fatal, 'fatal error latched');
  assert.match(rpc._fatal.message, /uncaught boom/);
  await assert.rejects(rpc.call(0, EMPTY), /uncaught boom/);
  assert.equal(wasm._calls.writeFrame, 0);
});

test('terminate() rejects pending work, latches, terminates worker once, and is idempotent', async () => {
  const { rpc, worker, wasm } = makeRig({ timeoutMs: 1000 });
  const p = rpc.call(0, EMPTY); // pending call (long timeout so it can't self-fire)
  rpc.terminate();
  await assert.rejects(p, /terminated/);
  assert.equal(worker.terminated, 1);

  // Idempotent: a second terminate() neither throws nor double-terminates.
  assert.doesNotThrow(() => rpc.terminate());
  assert.equal(worker.terminated, 1);

  // Subsequent calls fail immediately with the latched (terminated) error.
  await assert.rejects(rpc.call(0, EMPTY), /terminated/);
  assert.equal(wasm._calls.writeFrame, 1); // only the first call ever wrote
});

test('terminate() during init rejects _initPromise and terminates worker', async () => {
  const { rpc, worker } = makeRig({ timeoutMs: 1000, ready: false });
  rpc.terminate();
  await assert.rejects(rpc._initPromise, /terminated/);
  assert.equal(worker.terminated, 1);
  await assert.rejects(rpc.call(0, EMPTY), /terminated/); // still poisoned
});

// --- Rpc.create worker-leak protection (criterion 5) ----------------------

// Temporarily install global overrides; restore them after `fn` (awaited).
async function withGlobals(overrides, fn) {
  const keys = Object.keys(overrides);
  const saved = keys.map(k => [k, globalThis[k]]);
  for (const k of keys) globalThis[k] = overrides[k];
  try { return await fn(); } finally {
    for (const [k, v] of saved) globalThis[k] = v;
  }
}

test('Rpc.create terminates the Worker when _initPromise fails', async () => {
  const exports = {
    init_ring_buffers() {}, get_request_ptr: () => 0, get_response_ptr: () => 1024,
    get_buffer_size: () => 2048, get_scratch_ptr: () => 0, get_scratch_size: () => 512,
  };
  let created = null;
  // On receiving {type:'init'}, the fake worker immediately reports an error,
  // failing init deterministically (no timer wait).
  class InitErrorWorker extends FakeWorker {
    constructor() { super(); created = this; }
    postMessage(msg) { super.postMessage(msg); if (msg && msg.type === 'init') this.send({ type: 'error', message: 'init boom' }); }
  }
  await withGlobals(
    { Worker: InitErrorWorker, fetch: async () => ({ arrayBuffer: async () => new ArrayBuffer(0) }) },
    async () => {
      const oc = WebAssembly.compile, oi = WebAssembly.instantiate;
      WebAssembly.compile = async () => 'fakeModule';
      WebAssembly.instantiate = async () => ({ exports });
      try {
        await assert.rejects(
          Rpc.create({ mainWasmUrl: 'm', workerJsUrl: 'w', workerWasmUrl: 'p', timeoutMs: 1000 }),
          /init boom/,
        );
      } finally { WebAssembly.compile = oc; WebAssembly.instantiate = oi; }
    },
  );
  assert.ok(created, 'a Worker was created');
  assert.equal(created.terminated, 1, 'Worker terminated on init failure');
});

test('Rpc.create terminates the Worker when setup (fetch) fails', async () => {
  let created = null;
  class W extends FakeWorker { constructor() { super(); created = this; } }
  await withGlobals(
    { Worker: W, fetch: async () => { throw new Error('fetch boom'); } },
    async () => {
      await assert.rejects(
        Rpc.create({ mainWasmUrl: 'm', workerJsUrl: 'w', workerWasmUrl: 'p', timeoutMs: 1000 }),
        /fetch boom/,
      );
    },
  );
  assert.ok(created, 'a Worker was created');
  assert.equal(created.terminated, 1, 'Worker terminated on setup failure');
});
