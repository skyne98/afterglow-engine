# `afterglow-rpc-macros` API — the `#[rpc]` attribute

> Status: working; API checked against the 2026-07-10 source.

`#[rpc]` on a trait generates the Rust server trait, typed Rust client, and
(with `worker = Type`) the native spawn constructor and wasm worker exports.

See [`ring-buffer.md`](ring-buffer.md) for the transport the client talks over
and [`web-shared-memory.md`](web-shared-memory.md) for the wasm export contract
`worker.js` calls.

## Supported trait shape

```rust
use afterglow_rpc_macros::rpc;

#[rpc(worker = PhysicsWorker)]
pub trait Physics {
    fn step(state: Vec<f32>, dt: f32) -> Vec<f32>;
    fn apply_force(body_id: u32, fx: f32, fy: f32, fz: f32) -> bool;
}
```

The attribute is either empty (`#[rpc]`) or `#[rpc(worker = Type)]`, where
`Type` is the concrete server-impl type. Method declarations must be plain
`fn name(ident: Type, ...) -> Type;` — the macro injects `&mut self` into the
generated `<Name>Server` methods and preserves the trait's visibility.

Rejected at compile time (a `syn::Error`, never a panic):

- a `self` / `&self` / `&mut self` receiver;
- trait generics or a `where` clause, or any supertraits;
- `async`, `const`, `unsafe`, `extern`, or variadic (`...`) methods;
- generic methods, or methods with a `where` clause;
- associated consts or types, or methods with default bodies;
- non-identifier parameter patterns (e.g. `(x,): (u32,)`).

## Reserved method names

- `serve`, `new`, and `transport` are **always** reserved (the server dispatch,
  the client constructor, and the client transport accessor).
- `spawn_worker` is reserved **only when `worker = ...` is used** (the generated
  native client constructor). Without `worker`, a method named `spawn_worker`
  is allowed.

## Generated surface

For a trait `Physics` with `n` methods declared in order:

### `<Name>Server` trait

```rust
pub trait PhysicsServer {
    fn step(&mut self, state: Vec<f32>, dt: f32) -> Vec<f32>;
    fn apply_force(&mut self, body_id: u32, fx: f32, fy: f32, fz: f32) -> bool;
    /// Dispatch a `(method, args)` request to the matching method, returning
    /// the postcard-encoded result or an `RpcError`. Provided.
    fn serve(&mut self, method: u32, args: &[u8]) -> RpcResult<Vec<u8>> { .. }
}
```

Method ids are assigned `0, 1, …` in declaration order; `serve` matches on the
id and returns `RpcError::UnknownMethod` for any other id.

### `<Name>Client<T: Transport>`

```rust
pub struct PhysicsClient<T: Transport> { /* private transport */ }
impl<T: Transport> PhysicsClient<T> {
    pub fn new(t: T) -> Self;
    pub fn transport(&self) -> &T;
    pub fn step(&self, state: Vec<f32>, dt: f32) -> RpcResult<Vec<f32>>;
    pub fn apply_force(&self, body_id: u32, fx: f32, fy: f32, fz: f32) -> RpcResult<bool>;
}
```

Each method encodes its args as a postcard tuple (`(arg0, arg1, …)`), calls
`Transport::call(method_id, &args)`, and decodes the returned
payload. Trailing commas force tuple semantics, so a single-argument method
round-trips the same way as a multi-argument one. Fields stay private; use
`transport()` for ad-hoc/raw calls.

### Native spawn (only with `worker = Type`)

```rust
#[cfg(not(target_arch = "wasm32"))]
impl PhysicsClient<WorkerTransport> {
    pub fn spawn_worker(impl_: PhysicsWorker)
        -> RpcResult<(Self, EventReceiver)>;
}
```

Spawns the worker on an OS thread over 1 MiB request/response/event rings and
returns the typed client + event receiver. The worker thread is joined when the
client is dropped. See [`ring-buffer.md`](ring-buffer.md).

### Wasm exports (only with `worker = Type`, `#[cfg(target_arch = "wasm32")]`)

```text
afterglow_wasm_init() -> void
afterglow_wasm_serve_frame(method, args_ptr, args_len, out_ptr, out_max) -> i32
afterglow_wasm_input_ptr() -> usize
afterglow_wasm_input_size() -> usize   # 1 MiB
afterglow_wasm_output_ptr() -> usize
afterglow_wasm_output_size() -> usize  # 1 MiB
```

`afterglow_wasm_init` constructs `Type::default()`, so the worker type **must
implement `Default`**. `serve_frame` decodes/dispatches the method through the
generated server trait and always writes a postcard `Response` envelope to
`out_ptr`; it returns the encoded byte count, or `-1` if even a compact
oversized-response error envelope cannot fit. See
[`web-shared-memory.md`](web-shared-memory.md) for how `worker.js` calls these.

### Async wasm exports

For an `async fn` service the macro emits `afterglow_wasm_serve_async`,
`afterglow_wasm_tick`, and `afterglow_wasm_drain_completion`. Web clients own
256 fixed task slots. The wasm worker reserves a 256-entry `VecDeque` at init
and rejects growth past that capacity; `AsyncWorker.poll()` drains at most 32
responses per invocation. Promise-returning generated methods are convenience
adapters over these bounded numeric slots, not owners of an unbounded engine
queue.

## One worker service per wasm cdylib

The wasm exports use fixed `#[no_mangle]` names (`afterglow_wasm_*`), so **at
most one `#[rpc(worker = ...)]` service may be linked into a single wasm
cdylib**. Multiple non-worker `#[rpc]` traits (no `worker = …`) may coexist in
one module — they only generate `Server` / `Client` names and never
emit wasm symbols. (The demo's `multi_trait` test module exercises two
non-worker traits dispatching through the real native worker loop in one
module.)

## TypeScript boundary

The macro emits a typed `.client.ts` alongside each service. Generated clients
import authored runtime modules through `.ts` specifiers (`codec.ts`,
`async-worker.ts`, or `rpc.ts`); they never import generated JavaScript
artifacts. `scripts/build-web.ts` bundles deployment `.js` output and rejects
JavaScript specifiers in authored TypeScript. Unsupported Rust type shapes are
rejected during generation rather than silently falling back to untyped values.
