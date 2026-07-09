# afterglow-rpc

Ultra-fast, statically-typed RPC for main↔worker and worker↔worker calls.
Interfaces are defined **once in Rust**; a macro generates the Rust server +
client + a schema, and the build system (`xtask gen-ts`) generates a TypeScript
client. You talk to a worker as if it were a local object.

## Wire format

**[postcard]** (serde-based, compact, no schema bytes on the wire, `no_std`-friendly).
Frames are `(service: &str, method_id: u32, postcard(args))` → `postcard(ret)`.

## Define a worker in Rust

```rust
use afterglow_rpc_macros::rpc;

#[rpc]
pub trait Physics {
    fn step(state: Vec<f32>, dt: f32) -> Vec<f32>;
    fn apply_force(body_id: u32, fx: f32, fy: f32, fz: f32) -> bool;
}
```

The `#[rpc]` macro generates:
- `PhysicsServer` — the trait a worker implements.
- `PhysicsClient<T: Transport>` — a typed client; call methods as if local.
- `serve(server, method_id, args)` — server-side dispatch.
- `SCHEMA: RpcSchema` — for codegen.

## Use the Rust client

```rust
use afterglow_rpc::{Loopback, Transport};
let client = PhysicsClient::new(Loopback(|svc, method, args| {
    serve(&mut worker, method, args) // generated dispatch
}));
let next = client.step(vec![0.0, 1.0, 2.0], 0.5)?;  // -> Vec<f32>
```

`Transport` is the byte-pipe abstraction — one impl per link: in-process
channel (native CEF workers), `postMessage` (web workers), or the CEF IPC
bridge (host↔page). Generated clients are transport-agnostic.

## Generate the TypeScript client

```sh
cargo run -p xtask gen-ts
```

Runs the schema dump and emits `types/physics.ts`:

```ts
export class PhysicsClient {
  constructor(private readonly t: Transport) {}
  async step(state: number[], dt: number): Promise<number[]> {
    const resp = await this.t.call("Physics", 0, encode([state, dt]));
    return decode<number[]>(resp);
  }
  async apply_force(body_id: number, fx: number, fy: number, fz: number): Promise<boolean> { ... }
}
```

In the page/worker: `const physics = new PhysicsClient(postMessageTransport); physics.step(...)`.
The TS runtime provides `Transport` + a `postcard` codec (encode/decode).

## Crates

- `afterglow-rpc` — runtime: `Transport` trait, postcard codec, `RpcSchema`, `Loopback`.
- `afterglow-rpc-macros` — `#[rpc]` proc-macro (server trait + client + dispatch + schema).
- `afterglow-rpc-demo` — `Physics` example + round-trip test + `dump-schema` bin.

## Status

- Rust side: working + tested (client↔server over `Loopback`).
- TS client: generated from the schema; needs the TS `Transport` + `postcard`
  runtime (small, to be provided) and per-type codecs for custom structs
  (planned: a `#[derive(RpcType)]` that emits their `.ts` + codec, ts-rs-style).

[postcard]: https://github.com/jamesmunns/postcard
