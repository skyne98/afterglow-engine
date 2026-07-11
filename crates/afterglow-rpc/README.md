# afterglow-rpc

Ultra-fast, statically-typed RPC for main↔worker and worker↔worker calls in
afterglow-engine. Interfaces are defined **once in Rust**; the `#[rpc]` macro
generates the Rust server trait + typed client, and (for
worker services) the native thread spawn and the wasm exports. You call a
worker as if it were a local object.

There is **one payload communication mechanism**: the lock-free SPSC
[`RingBuffer`] over a 4-byte-aligned shared region, with `AtomicU32`
Acquire/Release indices. Wake-ups are separate and payload-free —
`Thread::unpark` natively and `postMessage('wake')` on the web. RPC values are
encoded with [postcard].

Two backends share the same framing and ring layout:

| Target | Worker type | Memory backing | Crate |
|--------|-------------|----------------|-------|
| Native | OS thread (`std::thread`) | Compact aligned heap allocation shared by `Arc` | `afterglow_rpc::native` |
| Web | Web Worker (wasm) | `SharedArrayBuffer`-backed `WebAssembly.Memory` | `afterglow-web` |

## Wire format

**[postcard]** (serde-based, compact, no schema bytes on the wire, `no_std`-friendly).
A request frame is `[method: u32 LE][postcard(args)]`; the response is a
postcard-encoded `Response` envelope so a success, a server error, a decode
failure, and a unit/zero-byte result are always distinguishable.

## Define a worker in Rust

```rust
use afterglow_rpc_macros::rpc;

/// `worker = PhysicsWorker` wires the concrete impl type, the native
/// `spawn_worker`, and the wasm exports. `PhysicsWorker: Default` is required.
#[rpc(worker = PhysicsWorker)]
pub trait Physics {
    fn step(state: Vec<f32>, dt: f32) -> Vec<f32>;
    fn apply_force(body_id: u32, fx: f32, fy: f32, fz: f32) -> bool;
}
```

The `#[rpc]` macro generates:

- `PhysicsServer` — the trait a worker implements; the macro injects `&mut self`
  and provides `serve(&mut self, method: u32, args: &[u8]) -> RpcResult<Vec<u8>>`
  dispatch.
- `PhysicsClient<T: Transport>` — a typed client; call methods as if local.
- With `worker = ...`: `PhysicsClient::spawn_worker` (native) + the wasm exports
  `afterglow_wasm_*` used by `afterglow-web`'s `worker.js`.

See [`docs/api/rpc-macro.md`](../../docs/api/rpc-macro.md) for the full macro
shape, reserved method names, and constraints.

## Use the Rust client (native)

```rust
use afterglow_rpc_demo::{PhysicsClient, PhysicsWorker};

let (client, events) = PhysicsClient::spawn_worker(PhysicsWorker)?;
let next = client.step(vec![0.0, 1.0, 2.0], 0.5)?;        // -> Vec<f32>
assert!(client.apply_force(3, 0.0, 9.8, 0.0)?);

let mut evs = Vec::new();
events.drain_into(&mut evs);
```

`spawn_worker` runs the worker on a native OS thread over 1 MiB request,
response, and event rings and returns the typed client + an `EventReceiver`.
The worker thread is joined when the client is dropped. `Transport::call` writes
`[method: u32 LE][postcard args]`, wakes the worker, bounded-waits for the
response, and unwraps the `Response` envelope. See
[`docs/api/ring-buffer.md`](../../docs/api/ring-buffer.md) for the ring layout,
the native halves, and the timeout-poison lifecycle.

## Web target

The page owns a shared `WebAssembly.Memory` backed by `SharedArrayBuffer`. An
`afterglow-web` wasm instance exposes the page-side request/response ring API;
a Web Worker receives the SAB once during init and reads/writes those rings
with matching `Atomics`/framing. `postMessage` is wake-up only — it never
carries a request, response, or event payload. See
[`docs/api/web-shared-memory.md`](../../docs/api/web-shared-memory.md) for the
exports, the JS client contract, and the web lifecycle.

## JavaScript boundary

The generated client is Rust-only. The browser API is deliberately a low-level
`call(method_id, encoded_args)` byte transport; applications own their JS/TS
value codecs. The demo's hand-written
[`rpc.js`](../afterglow-web/www/rpc.js) helpers cover only its `Physics` types
and do not pretend to be a general Rust-to-TypeScript schema generator.

## Crates

- `afterglow-rpc` — runtime: `RingBuffer`, `Transport` trait, postcard codec,
  `Response` envelope and the `native` module
  (`RingStorage`, `spawn_worker_loop`, `run_worker_loop`, events).
- `afterglow-rpc-macros` — the `#[rpc]` proc-macro (server trait + client +
  dispatch + native spawn + wasm exports).
- `afterglow-rpc-demo` — demo `Physics` service + `bench_rpc` stress test.
- `afterglow-web` — web wasm target + `www/rpc.js` + `www/worker.js`.

## Status

- Native: working and tested (ring buffers, worker transport, timeout-poison
  regression, events, end-to-end service RPC). Run the benchmark with
  `cargo run --release --example bench_rpc -p afterglow-rpc-demo`.
- Web: working; `www/rpc.js` + `www/worker.js` drive the shared-memory rings.

[postcard]: https://github.com/jamesmunns/postcard
