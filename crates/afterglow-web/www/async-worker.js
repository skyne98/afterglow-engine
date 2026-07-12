// Async worker glue for afterglow-engine wasm workers with `async fn` methods.
//
// This drives the poll model on the web target:
// - `serve_async(method, args, task_id)` spawns a task on the wasm executor.
// - `tick()` drives the executor (re-polls pending tasks, including fetches).
// - `drain_completion()` pops a `[task_id][Response]` from the completion queue.
//
// It also provides the `ag_fetch_start` / `ag_fetch_poll` imports that the
// asset loader worker uses to fetch assets via JS (wasm can't fetch directly).
//
// The render thread uses the generated TS client (e.g. `AssetLoaderClient`),
// which calls `call()` under the hood. `AsyncWorker` implements `RpcTransport`:
// `call(method, args)` returns a Promise that resolves on a later `poll()`.

/// A pending JS fetch (full GET), keyed by `fetch_id`.
class PendingFetch {
  constructor(url) {
    this.promise = fetch(url);
    this.resolved = false;
    this.bytes = null;
    this.error = null;
    this.promise
      .then(async (resp) => {
        if (!resp.ok) {
          this.error = new Error(`fetch ${resp.status}: ${url}`);
        } else {
          this.bytes = new Uint8Array(await resp.arrayBuffer());
        }
        this.resolved = true;
      })
      .catch((e) => {
        this.error = e;
        this.resolved = true;
      });
  }
}

/// A pending HEAD fetch (to get Content-Length).
class HeadFetch {
  constructor(url) {
    this.promise = fetch(url, { method: 'HEAD' });
    this.resolved = false;
    this.contentLength = null;
    this.error = null;
    this.promise
      .then((resp) => {
        if (!resp.ok) {
          this.error = new Error(`HEAD ${resp.status}: ${url}`);
        } else {
          const cl = resp.headers.get('Content-Length');
          this.contentLength = cl ? parseInt(cl, 10) : null;
        }
        this.resolved = true;
      })
      .catch((e) => {
        this.error = e;
        this.resolved = true;
      });
  }
}

/// A pending ranged GET fetch.
class RangeFetch {
  constructor(url, offset, len) {
    const end = offset + len - 1;
    this.promise = fetch(url, { headers: { Range: `bytes=${offset}-${end}` } });
    this.resolved = false;
    this.bytes = null;
    this.error = null;
    this.promise
      .then(async (resp) => {
        if (!resp.ok && resp.status !== 206) {
          this.error = new Error(`range fetch ${resp.status}: ${url}`);
        } else {
          this.bytes = new Uint8Array(await resp.arrayBuffer());
        }
        this.resolved = true;
      })
      .catch((e) => {
        this.error = e;
        this.resolved = true;
      });
  }
}

/// The async worker driver. Instantiate the wasm module, then drive the
/// executor + drain completions. Implements `RpcTransport` so generated TS
/// clients (`AssetLoaderClient`, etc.) can use it directly.
export class AsyncWorker {
  /// @param {WebAssembly.Exports} wasm — the instantiated wasm exports.
  /// @param {string} baseUrl — base URL for fetch (e.g. '' for same-origin).
  constructor(wasm, baseUrl = '') {
    this.w = wasm;
    this.baseUrl = baseUrl;
    this.nextFetchId = 1;
    this.pendingFetches = new Map();
    this._pendingCalls = new Map(); // task_id → { resolve, reject }
    this._taskIdCounter = 0;
  }

  /// `RpcTransport.call`: spawn an async task and return a Promise that
  /// resolves when `poll()` delivers the completion. This is what generated TS
  /// clients call under the hood.
  /// @param {number} method
  /// @param {Uint8Array} args
  /// @returns {Promise<Uint8Array>}
  async call(method, args) {
    const taskId = this.serveAsync(method, args);
    if (taskId < 0) throw new Error('async worker: serveAsync failed');
    return new Promise((resolve, reject) => {
      this._pendingCalls.set(taskId, { resolve, reject });
    });
  }

  /// Spawn an async task: write args to the wasm input scratch, call
  /// `serve_async(method, args, task_id)`. Returns the `task_id`.
  /// @param {number} method
  /// @param {Uint8Array} args
  /// @returns {number} task_id (or -1 on error)
  serveAsync(method, args) {
    const taskId = this._nextTaskId();
    const inPtr = this.w.afterglow_wasm_input_ptr();
    const inSize = this.w.afterglow_wasm_input_size();
    if (args.length + 12 > inSize) {
      console.error('async worker: args too large for input scratch');
      return -1;
    }
    // Write [method:u32 LE][task_id:u64 LE][args] to the input scratch.
    const view = new DataView(this.w.memory.buffer, inPtr, 12 + args.length);
    view.setUint32(0, method, true);
    view.setBigUint64(4, BigInt(taskId), true);
    new Uint8Array(this.w.memory.buffer, inPtr + 12, args.length).set(args);
    // Call serve_async with the input scratch (it reads method+task_id+args).
    // Actually the exported fn takes (method, args_ptr, args_len, task_id).
    const r = this.w.afterglow_wasm_serve_async(method, inPtr + 12, args.length, BigInt(taskId));
    if (r < 0) return -1;
    return taskId;
  }

  /// Drive the executor + drain completions + resolve pending promises. Call
  /// this each frame (or on a timer). Returns the raw completions array for
  /// callers that want to inspect them.
  /// @returns {Array<{taskId: number, response: Uint8Array}>}
  poll() {
    // 1. Tick the executor (re-polls pending tasks, including fetches).
    this.w.afterglow_wasm_tick();

    // 2. Drain completions and resolve pending calls.
    const completions = [];
    const outPtr = this.w.afterglow_wasm_output_ptr();
    const outSize = this.w.afterglow_wasm_output_size();
    for (;;) {
      const n = this.w.afterglow_wasm_drain_completion(outPtr, outSize);
      if (n < 0) break; // -1 = none, -2 = too small (shouldn't happen with 1MiB)
      const bytes = new Uint8Array(this.w.memory.buffer, outPtr, n).slice();
      // Parse [task_id:u64 LE][Response envelope].
      const dv = new DataView(bytes.buffer, bytes.byteOffset, 8);
      const taskId = Number(dv.getBigUint64(0, true));
      const responseBytes = bytes.subarray(8);
      completions.push({ taskId, response: responseBytes });
      // Resolve the pending call for this task_id.
      const pending = this._pendingCalls.get(taskId);
      if (pending) {
        this._pendingCalls.delete(taskId);
        try {
          pending.resolve(responseBytes);
        } catch (e) {
          pending.reject(e);
        }
      }
    }
    return completions;
  }

  /// JS import: start a fetch. Called from wasm via `ag_fetch_start`.
  /// @param {number} urlPtr
  /// @param {number} urlLen
  /// @returns {number} fetch_id (>0) or 0 on error
  fetchStart(urlPtr, urlLen) {
    const url = new TextDecoder().decode(
      new Uint8Array(this.w.memory.buffer, urlPtr, urlLen)
    );
    const fullUrl = this._resolveUrl(url);
    const id = this.nextFetchId++;
    this.pendingFetches.set(id, new PendingFetch(fullUrl));
    return id;
  }

  /// JS import: poll a fetch. Called from wasm via `ag_fetch_poll`.
  /// @param {number} fetchId
  /// @param {number} outPtr
  /// @param {number} outMax
  /// @returns {number} -1 pending, >=0 byte count (complete), -2 out too small
  fetchPoll(fetchId, outPtr, outMax) {
    const pending = this.pendingFetches.get(fetchId);
    if (!pending) return -1;
    if (!pending.resolved) return -1;
    this.pendingFetches.delete(fetchId);
    if (pending.error) {
      // Write an empty response — the wasm side handles null/error.
      return 0;
    }
    if (pending.bytes.length > outMax) return -2;
    new Uint8Array(this.w.memory.buffer, outPtr, outMax).set(pending.bytes);
    return pending.bytes.length;
  }

  /// JS import: start a HEAD fetch (to get Content-Length). Called from wasm
  /// via `ag_fetch_head_start`.
  /// @param {number} urlPtr
  /// @param {number} urlLen
  /// @returns {number} fetch_id (>0) or 0 on error
  headStart(urlPtr, urlLen) {
    const url = new TextDecoder().decode(
      new Uint8Array(this.w.memory.buffer, urlPtr, urlLen)
    );
    const fullUrl = this._resolveUrl(url);
    const id = this.nextFetchId++;
    this.pendingFetches.set(id, new HeadFetch(fullUrl));
    return id;
  }

  /// JS import: poll a HEAD fetch. Writes Content-Length as u64 LE to `out`.
  /// @param {number} fetchId
  /// @param {number} outPtr
  /// @param {number} outMax
  /// @returns {number} -1 pending, 8 complete, -2 error
  headPoll(fetchId, outPtr, outMax) {
    const pending = this.pendingFetches.get(fetchId);
    if (!pending) return -2;
    if (!pending.resolved) return -1;
    this.pendingFetches.delete(fetchId);
    if (pending.error || pending.contentLength === null) return -2;
    const buf = new ArrayBuffer(8);
    new DataView(buf).setBigUint64(0, BigInt(pending.contentLength), true);
    new Uint8Array(this.w.memory.buffer, outPtr, 8).set(new Uint8Array(buf));
    return 8;
  }

  /// JS import: start a ranged GET fetch. Called from wasm via `ag_fetch_range_start`.
  /// @param {number} urlPtr
  /// @param {number} urlLen
  /// @param {number} offset
  /// @param {number} len
  /// @returns {number} fetch_id (>0) or 0 on error
  rangeStart(urlPtr, urlLen, offset, len) {
    const url = new TextDecoder().decode(
      new Uint8Array(this.w.memory.buffer, urlPtr, urlLen)
    );
    const fullUrl = this._resolveUrl(url);
    const id = this.nextFetchId++;
    this.pendingFetches.set(id, new RangeFetch(fullUrl, offset, len));
    return id;
  }

  // --- private ---

  _nextTaskId() {
    return ++this._taskIdCounter;
  }

  _resolveUrl(path) {
    if (!this.baseUrl) return path;
    const p = path.startsWith('/') ? path.slice(1) : path;
    const sep = this.baseUrl.endsWith('/') ? '' : '/';
    return `${this.baseUrl}${sep}${p}`;
  }
}

/// The wasm import object for an async worker. Provides `memory` (shared) and
/// the `ag_fetch_start` / `ag_fetch_poll` imports the asset worker uses.
/// @param {AsyncWorker} driver
/// @param {WebAssembly.Memory} memory
export function asyncWorkerImports(driver, memory) {
  return {
    env: {
      memory,
      notify_worker: () => {}, // wake-up only (not used by async workers)
      ag_fetch_start: (urlPtr, urlLen) => driver.fetchStart(urlPtr, urlLen),
      ag_fetch_poll: (fetchId, outPtr, outMax) => driver.fetchPoll(fetchId, outPtr, outMax),
      ag_fetch_head_start: (urlPtr, urlLen) => driver.headStart(urlPtr, urlLen),
      ag_fetch_head_poll: (fetchId, outPtr, outMax) => driver.headPoll(fetchId, outPtr, outMax),
      ag_fetch_range_start: (urlPtr, urlLen, offset, len) => driver.rangeStart(urlPtr, urlLen, offset, len),
    },
  };
}
