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

import { unwrapResponse } from './codec.ts';

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
    const start = Number(offset);
    const end = start + Number(len) - 1;
    this.promise = fetch(url, { headers: { Range: `bytes=${start}-${end}` } });
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
    this._memory = null; // set by asyncWorkerImports
    this.nextFetchId = 1;
    this._fetchCapacity = 256;
    this._fetchIds = new Float64Array(this._fetchCapacity);
    this._fetchIds.fill(-1);
    this._fetches = new Array(this._fetchCapacity).fill(null);
    this._pendingFetchCount = 0;
    this._callCapacity = 256;
    this._callIds = new Float64Array(this._callCapacity);
    this._callIds.fill(-1);
    this._callResolves = new Array(this._callCapacity).fill(null);
    this._callRejects = new Array(this._callCapacity).fill(null);
    this._pendingCallCount = 0;
    this._taskIdCounter = 0;
    this._pumpScheduled = false;
    this._completionLimit = 32;
    this._lastPollCompletions = 0;
    this._totalCompletions = 0;
    this._completionLimitHits = 0;
  }

  /// `RpcTransport.call`: spawn an async task and return a Promise that
  /// resolves when `poll()` delivers the completion. This is what generated TS
  /// clients call under the hood.
  /// @param {number} method
  /// @param {Uint8Array} args
  /// @returns {Promise<Uint8Array>}
  async call(method, args) {
    const taskId = this._nextTaskId();
    const slot = taskId % this._callCapacity;
    if (this._callIds[slot] !== -1) throw new Error('async worker: fixed task capacity exhausted');
    return new Promise((resolve, reject) => {
      this._callIds[slot] = taskId;
      this._callResolves[slot] = resolve;
      this._callRejects[slot] = reject;
      this._pendingCallCount++;
      if (this.serveAsync(method, args, taskId) < 0) {
        this._releaseCallSlot(slot);
        reject(new Error('async worker: serveAsync failed'));
        return;
      }
      this._schedulePump();
    });
  }

  // Exactly one page-thread pump serves all pending calls. This avoids the
  // previous one-setTimeout-loop-per-RPC event storm under sustained streaming.
  _schedulePump() {
    if (this._pumpScheduled || this._pendingCallCount === 0) return;
    this._pumpScheduled = true;
    setTimeout(() => {
      this._pumpScheduled = false;
      if (this._pendingCallCount === 0) return;
      this.poll();
      this._schedulePump();
    }, 0);
  }

  /// Spawn an async task: write args to the wasm input scratch, call
  /// `serve_async(method, args, task_id)`. Returns the `task_id`.
  /// @param {number} method
  /// @param {Uint8Array} args
  /// @returns {number} task_id (or -1 on error)
  serveAsync(method, args, taskId = this._nextTaskId()) {
    const inPtr = this.w.afterglow_wasm_input_ptr();
    const inSize = this.w.afterglow_wasm_input_size();
    if (args.length + 12 > inSize) {
      console.error('async worker: args too large for input scratch');
      return -1;
    }
    // Write [method:u32 LE][task_id:u64 LE][args] to the input scratch.
    const view = new DataView((this._memory || this.w.memory).buffer, inPtr, 12 + args.length);
    view.setUint32(0, method, true);
    view.setBigUint64(4, BigInt(taskId), true);
    new Uint8Array((this._memory || this.w.memory).buffer, inPtr + 12, args.length).set(args);
    // Call serve_async with the input scratch (it reads method+task_id+args).
    // Actually the exported fn takes (method, args_ptr, args_len, task_id).
    const r = this.w.afterglow_wasm_serve_async(method, inPtr + 12, args.length, BigInt(taskId));
    if (r < 0) return -1;
    return taskId;
  }

  /// Drive the executor and resolve a bounded number of completions. The
  /// Promise API is game-facing; engine ownership remains in fixed task slots.
  /// @returns {number} number of completions drained
  poll(maxCompletions = this._completionLimit) {
    this.w.afterglow_wasm_tick();
    const outPtr = this.w.afterglow_wasm_output_ptr();
    const outSize = this.w.afterglow_wasm_output_size();
    const memory = this._memory || this.w.memory;
    let drained = 0;
    while (drained < maxCompletions) {
      const n = this.w.afterglow_wasm_drain_completion(outPtr, outSize);
      if (n < 0) break;
      if (n < 8) continue;
      const taskId = Number(new DataView(memory.buffer, outPtr, 8).getBigUint64(0, true));
      // Own only the response envelope. The next drain overwrites wasm output.
      const responseBytes = new Uint8Array(memory.buffer, outPtr + 8, n - 8).slice();
      drained++;
      const slot = taskId % this._callCapacity;
      if (this._callIds[slot] === taskId) {
        const resolve = this._callResolves[slot];
        const reject = this._callRejects[slot];
        this._releaseCallSlot(slot);
        try {
          resolve(unwrapResponse(responseBytes));
        } catch (e) {
          reject(e);
        }
      }
    }
    this._lastPollCompletions = drained;
    this._totalCompletions += drained;
    if (drained === maxCompletions) this._completionLimitHits++;
    return drained;
  }

  /// JS import: start a fetch. Called from wasm via `ag_fetch_start`.
  /// @param {number} urlPtr
  /// @param {number} urlLen
  /// @returns {number} fetch_id (>0) or 0 on error
  fetchStart(urlPtr, urlLen) {
    const url = new TextDecoder().decode(
      Uint8Array.from(new Uint8Array((this._memory || this.w.memory).buffer, urlPtr, urlLen))
    );
    const fullUrl = this._resolveUrl(url);
    return this._registerFetch(new PendingFetch(fullUrl));
  }

  /// JS import: poll a fetch. Called from wasm via `ag_fetch_poll`.
  /// @param {number} fetchId
  /// @param {number} outPtr
  /// @param {number} outMax
  /// @returns {number} -1 pending, >=0 byte count (complete), -2 out too small
  fetchPoll(fetchId, outPtr, outMax) {
    const pending = this._getFetch(fetchId);
    if (!pending) return -1;
    if (!pending.resolved) return -1;
    this._releaseFetch(fetchId);
    if (pending.error) {
      // Write an empty response — the wasm side handles null/error.
      return 0;
    }
    if (pending.bytes.length > outMax) return -2;
    new Uint8Array((this._memory || this.w.memory).buffer, outPtr, outMax).set(pending.bytes);
    return pending.bytes.length;
  }

  /// JS import: start a HEAD fetch (to get Content-Length). Called from wasm
  /// via `ag_fetch_head_start`.
  /// @param {number} urlPtr
  /// @param {number} urlLen
  /// @returns {number} fetch_id (>0) or 0 on error
  headStart(urlPtr, urlLen) {
    const url = new TextDecoder().decode(
      Uint8Array.from(new Uint8Array((this._memory || this.w.memory).buffer, urlPtr, urlLen))
    );
    const fullUrl = this._resolveUrl(url);
    return this._registerFetch(new HeadFetch(fullUrl));
  }

  /// JS import: poll a HEAD fetch. Writes Content-Length as u64 LE to `out`.
  /// @param {number} fetchId
  /// @param {number} outPtr
  /// @param {number} outMax
  /// @returns {number} -1 pending, 8 complete, -2 error
  headPoll(fetchId, outPtr, outMax) {
    const pending = this._getFetch(fetchId);
    if (!pending) return -2;
    if (!pending.resolved) return -1;
    this._releaseFetch(fetchId);
    if (pending.error || pending.contentLength === null || outMax < 8) return -2;
    new DataView((this._memory || this.w.memory).buffer, outPtr, 8)
      .setBigUint64(0, BigInt(pending.contentLength), true);
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
      Uint8Array.from(new Uint8Array((this._memory || this.w.memory).buffer, urlPtr, urlLen))
    );
    const fullUrl = this._resolveUrl(url);
    return this._registerFetch(new RangeFetch(fullUrl, offset, len));
  }

  // --- private ---

  _registerFetch(fetch) {
    for (let probe = 0; probe < this._fetchCapacity; probe++) {
      const id = this.nextFetchId++;
      const slot = id % this._fetchCapacity;
      if (this._fetchIds[slot] !== -1) continue;
      this._fetchIds[slot] = id;
      this._fetches[slot] = fetch;
      this._pendingFetchCount++;
      return id;
    }
    return 0;
  }

  _getFetch(id) {
    const slot = id % this._fetchCapacity;
    return this._fetchIds[slot] === id ? this._fetches[slot] : null;
  }

  _releaseFetch(id) {
    const slot = id % this._fetchCapacity;
    if (this._fetchIds[slot] !== id) return;
    this._fetchIds[slot] = -1;
    this._fetches[slot] = null;
    this._pendingFetchCount--;
  }

  _releaseCallSlot(slot) {
    this._callIds[slot] = -1;
    this._callResolves[slot] = null;
    this._callRejects[slot] = null;
    this._pendingCallCount--;
  }

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
  driver._memory = memory; // store for AsyncWorker methods that need buffer access
  return {
    env: {
      memory,
      notify_worker: () => {}, // wake-up only (not used by async workers)
      performance_now: () => performance.now(), // for benchmark functions
      ag_fetch_start: (urlPtr, urlLen) => driver.fetchStart(urlPtr, urlLen),
      ag_fetch_poll: (fetchId, outPtr, outMax) => driver.fetchPoll(fetchId, outPtr, outMax),
      ag_fetch_head_start: (urlPtr, urlLen) => driver.headStart(urlPtr, urlLen),
      ag_fetch_head_poll: (fetchId, outPtr, outMax) => driver.headPoll(fetchId, outPtr, outMax),
      ag_fetch_range_start: (urlPtr, urlLen, offset, len) => driver.rangeStart(urlPtr, urlLen, offset, len),
    },
  };
}
