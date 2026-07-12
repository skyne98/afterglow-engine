# Lifecycle & Errors

Both transports share the same lifecycle concerns: a call can time out, and a
timed-out call must never let a late reply be consumed as a later call's result.
The native and web transports solve these with parallel mechanisms.

## The timeout

| Transport | Deadline | Mechanism |
|---|---|---|
| Native (`WorkerTransport`) | `RESPONSE_DEADLINE` (10 s) | Parks with bounded retries until a response arrives |
| Web (`Rpc`) | `timeoutMs` (default 5000 ms) | Wake on the response, bounded by the timeout |

Both timeouts are injectable for tests: native tests pass a shorter
`response_deadline`; the web client takes `timeoutMs` in `Rpc.create`.

## The poison guarantee (native)

A timed-out native call must never let a late reply be consumed as a *later*
call's result. `WorkerTransport` latches a permanent **poison flag**:

- `call` first checks the poison flag and `JoinHandle::is_finished()`. If either
  is set, it returns `WorkerDead` immediately — without writing a request or
  reading the response ring.
- A call whose bounded wait elapses with no response returns `Timeout` and
  latches the poison flag (a `Release` store). The poison is **permanent** for
  the life of the transport — it is never cleared.
- Thereafter every `call` returns `WorkerDead` immediately, so a late reply to
  the timed-out call stays unconsumed rather than being mistaken for a later
  call's reply.
- If the worker thread has exited (`JoinHandle` is finished), `call` returns
  `WorkerDead` *without* poisoning — there is no late reply to worry about.

## The fatal latch (web)

The web client solves the same problem with a **fatal latch**. The first fatal
failure (init/response timeout, worker `onerror`/`error`, or `terminate()`) is
latched permanently into `_fatal` (idempotent: the first failure wins). Once
latched:

- every later `call` rejects immediately with the latched `Error`;
- late `ready` / response wakes are dropped — so a late reply to a timed-out
  call can never be consumed as a later call's result.

## Recovery

| Transport | Recover by |
|---|---|
| Native | Drop the transport and spawn a fresh worker (`spawn_worker` again). |
| Web | Call `terminate()` (or let a failed `create()` clean up), then construct a new `Rpc`. Its `init_ring_buffers` resets both rings. |

In both cases the ring allocations survive: native rings are owned by `Arc` (a
detached-after-deadline thread is safe because the allocations stay valid); web
rings are in the shared wasm memory, which `init_ring_buffers` resets.

## Shutdown

- **Native:** dropping the transport writes an empty request frame as the
  shutdown control, wakes the worker, waits up to 2 s, and joins it. A worker
  method that never returns cannot be forcibly cancelled; after the deadline its
  thread is detached safely.
- **Web:** `terminate()` latches a failure (idempotent if already latched) and
  stops the worker exactly once via an internal `_terminated` guard, so it is
  safe to call from cleanup paths and from a failed `create`.

## `RpcError`

`RpcError` covers the full failure space: codec/decode failures, unknown
methods, full/empty/undersized rings, corrupt frames/backing, transport/server
failures, a dead worker, response timeout, and I/O errors. The `Response`
envelope keeps a successful zero-byte/unit return distinguishable from an error.

## Thread safety (async singleton)

When using `#[rpc(worker = Type, singleton)]`, the `AsyncWorkerTransport` is
`Send + Sync` and shared via `Arc`. Multiple threads can call `call_async` and
`poll` concurrently — the request/response ring halves are behind `Mutex`es
(microsecond-level holds; never held during an `await`). The `task_id` is
`AtomicU64`, and the pending-futures map is `Mutex`-protected. This is what
makes the singleton asset loader safe for N threads to call simultaneously.

## Next

- [Performance](./performance.md) — what latency/bandwidth to expect.
- [Native Workers](./native-workers.md) / [Web Workers](./web-workers.md).
