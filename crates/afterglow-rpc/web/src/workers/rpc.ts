// afterglow-rpc main-thread RPC client.
//
// Instantiates the shared transport wasm + a Web Worker. `call(method, args)`
// writes `[method:u32][args]` to the request ring (the wasm `write_frame` auto-
// wakes the worker via the imported `notify_worker`), awaits the worker's
// response wake (postMessage carries NO payload — the ring is the transport),
// reads the response ring, and unwraps the postcard `Response`. One in-flight
// call at a time (SPSC). No busy-spinning: the main thread awaits a message.
//
// The postcard codec + `Response` envelope live in `codec.ts` (single source).

import { unwrapResponse } from './codec.ts';

const TIMEOUT_MS = 5000;

type CallResolve = (value: Uint8Array | PromiseLike<Uint8Array>) => void;
type CallReject = (reason?: unknown) => void;
type InitResolve = (value?: void | PromiseLike<void>) => void;

interface RpcWasm {
  init_ring_buffers(): void;
  get_request_ptr(): number;
  get_response_ptr(): number;
  get_buffer_size(): number;
  get_scratch_ptr(): number;
  get_scratch_size(): number;
  write_frame(ptr: number, len: number): number;
  read_response(ptr: number, maxLen: number): number;
}

interface RpcCreateOptions {
  mainWasmUrl: string;
  workerJsUrl: string;
  workerWasmUrl: string;
  timeoutMs?: number;
  workerInit?: unknown;
}

interface RpcOptions {
  timeoutMs?: number;
}

interface PendingCall {
  resolve: CallResolve;
  reject: CallReject;
  timer: ReturnType<typeof setTimeout>;
}

type WorkerMessage = {
  type?: string;
  message?: string;
};

export class Rpc {
  w: RpcWasm;
  mem: WebAssembly.Memory;
  worker: Worker;
  scratch: number;
  scratchLen: number;
  pending: PendingCall | null;
  _resolve: InitResolve | null;
  _reject: CallReject | null;
  _fatal: Error | null;
  _terminated: boolean;
  timeoutMs: number;
  _initPromise: Promise<void>;
  _initTimer: ReturnType<typeof setTimeout>;

  /// Instantiate the shared main wasm + spawn the worker; resolve once the
  /// worker reports ready. `notify_worker` is wired to wake the worker.
  static async create({ mainWasmUrl, workerJsUrl, workerWasmUrl, timeoutMs, workerInit = null }: RpcCreateOptions): Promise<Rpc> {
    const memory = new WebAssembly.Memory({ shared: true, initial: 256, maximum: 1024 });
    const worker = new Worker(workerJsUrl, { type: 'module' });
    let rpc: Rpc | null = null;
    try {
      const { exports: rawWasm } = await WebAssembly.instantiate(
        await WebAssembly.compile(await (await fetch(mainWasmUrl)).arrayBuffer()),
        { env: { memory, notify_worker: (): void => { worker.postMessage('wake'); } } },
      );
      const wasm = rawWasm as unknown as RpcWasm;
      wasm.init_ring_buffers();
      rpc = new Rpc(wasm, memory, worker, { timeoutMs });
      worker.postMessage({
        type: 'init', sab: memory.buffer,
        reqBase: wasm.get_request_ptr(), respBase: wasm.get_response_ptr(),
        bufSize: wasm.get_buffer_size(), wasmUrl: workerWasmUrl,
        workerInit,
      });
      await rpc._initPromise;
      worker.postMessage({ type: 'run' });
      return rpc;
    } catch (e) {
      // Setup or _initPromise failed: never leak a worker. terminate() latches
      // failure (idempotent if already poisoned) and stops the worker once.
      if (rpc) rpc.terminate(); else worker.terminate();
      throw e;
    }
  }

  constructor(wasm: RpcWasm, memory: WebAssembly.Memory, worker: Worker, opts: RpcOptions = {}) {
    this.w = wasm;
    this.mem = memory;
    this.worker = worker;
    this.scratch = wasm.get_scratch_ptr();
    this.scratchLen = wasm.get_scratch_size();
    this.pending = null;
    this._resolve = null;
    this._reject = null;
    this._fatal = null;        // latched transport failure (poison); null = healthy
    this._terminated = false;  // guard so worker.terminate() runs at most once
    this.timeoutMs = opts.timeoutMs ?? TIMEOUT_MS;
    this._initPromise = new Promise<void>((res, rej) => { this._resolve = res; this._reject = rej; });
    this._initTimer = setTimeout(() => this._fail(new Error('worker init timeout')), this.timeoutMs);
    worker.onmessage = (e: MessageEvent<unknown>) => this._onmsg(e.data);
    worker.onerror = (e: ErrorEvent) => this._fail(new Error('worker: ' + ((e && e.message) || e)));
  }

  _onmsg(d: unknown): void {
    // Poisoned: drop everything. A late response wake must never be read as a
    // later call's result, and a late ready/error must not resurrect state.
    if (this._fatal) return;
    const message = typeof d === 'object' && d !== null ? d as WorkerMessage : null;
    if (message && message.type === 'ready') { clearTimeout(this._initTimer); const r = this._resolve; this._resolve = this._reject = null; if (r) r(); return; }
    if (message && message.type === 'error') { this._fail(this._reject ? new Error('worker init: ' + (message.message || 'error')) : new Error(message.message || 'worker error')); return; }
    if (this.pending) this._readResponse(); // response wake (no payload)
  }

  /// One in-flight SPSC call: write [method][args] to scratch, write_frame
  /// (auto-wakes the worker), then await the response wake.
  async call(method: number, args: Uint8Array): Promise<Uint8Array> {
    if (this._fatal) throw this._fatal; // poisoned: never touch the rings again
    if (this.pending) throw new Error('RPC busy: one in-flight call at a time');
    const len = 4 + args.length;
    if (len > this.scratchLen) throw new Error('request too large for scratch');
    const view = new Uint8Array(this.mem.buffer, this.scratch, len);
    view[0] = method & 0xff;
    view[1] = (method >>> 8) & 0xff;
    view[2] = (method >>> 16) & 0xff;
    view[3] = (method >>> 24) & 0xff;
    view.set(args, 4);
    if (this.w.write_frame(this.scratch, len) !== 0) throw new Error('write_frame failed (ring full)');
    return new Promise<Uint8Array>((resolve, reject) => {
      const timer = setTimeout(() => this._fail(new Error('RPC timeout')), this.timeoutMs);
      this.pending = { resolve, reject, timer };
    });
  }

  _readResponse(): void {
    const n = this.w.read_response(this.scratch, this.scratchLen);
    const p = this.pending;
    this.pending = null;
    if (p) clearTimeout(p.timer);
    if (!p) return;
    if (n < 0) { p.reject(new Error('read_response returned ' + n)); return; }
    try { p.resolve(unwrapResponse(new Uint8Array(this.mem.buffer, this.scratch, n))); }
    catch (e) { p.reject(e); }
  }

  // Latch a fatal transport failure permanently. Idempotent: the first failure
  // wins; later _fail/terminate calls and late wakes are no-ops. Rejects the
  // init promise if still pending and/or the pending call if any. With no
  // pending work the error is still retained in `_fatal` so the next call fails
  // immediately.
  _fail(err: Error): void {
    if (this._fatal) return;
    this._fatal = err;
    clearTimeout(this._initTimer);
    if (this._reject) { const r = this._reject; this._resolve = this._reject = null; r(err); }
    if (this.pending) { const p = this.pending; this.pending = null; clearTimeout(p.timer); p.reject(err); }
  }

  terminate(): void {
    this._fail(new Error('terminated'));
    if (!this._terminated) { this._terminated = true; this.worker.terminate(); }
  }
}
