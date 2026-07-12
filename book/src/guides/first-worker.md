# Your First Worker

Define a worker service from scratch, build it for both backends, and call it
from both Rust and JavaScript.

## 1. Define the service

A worker service is a trait annotated with `#[rpc(worker = Type)]`, where `Type`
is the concrete server-impl type. Put this in a library crate that compiles to
both a native `rlib` and a wasm `cdylib`:

```toml
[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
afterglow-rpc = { path = "../afterglow-rpc" }
afterglow-rpc-macros = { path = "../afterglow-rpc-macros" }
```

```rust
use afterglow_rpc_macros::rpc;

#[rpc(worker = PhysicsWorker)]
pub trait Physics {
    /// Advance a body's state by dt; returns the new state.
    fn step(state: Vec<f32>, dt: f32) -> Vec<f32>;
    /// Apply a force to a body; returns whether it was accepted.
    fn apply_force(body_id: u32, fx: f32, fy: f32, fz: f32) -> bool;
}

/// The concrete server. `#[derive(Default)]` is required for the wasm path
/// (`afterglow_wasm_init` constructs `Type::default()`).
#[derive(Default)]
pub struct PhysicsWorker;

impl PhysicsServer for PhysicsWorker {
    fn step(&mut self, mut state: Vec<f32>, dt: f32) -> Vec<f32> {
        for v in state.iter_mut() {
            *v += dt;
        }
        state
    }
    fn apply_force(&mut self, body_id: u32, fx: f32, fy: f32, fz: f32) -> bool {
        body_id % 2 == 1 && (fx + fy + fz).abs() > 0.0
    }
}
```

The macro generates `PhysicsServer` (with a provided `serve` dispatcher),
`PhysicsClient<T: Transport>`, the native `spawn_worker` constructor, and the
wasm `afterglow_wasm_*` exports. Method ids are `0, 1, …` in declaration order,
so `step` is method `0` and `apply_force` is method `1`. See
[Defining a Service](../workers/defining-a-service.md) for the full rules.

## 2. Call it from Rust (native)

```rust
use afterglow_rpc_demo::{PhysicsClient, PhysicsWorker};

let (client, events) = PhysicsClient::spawn_worker(PhysicsWorker)?;

let next = client.step(vec![0.0, 1.0, 2.0], 0.5)?;   // → [0.5, 1.5, 2.5]
assert!(client.apply_force(3, 0.0, 9.8, 0.0)?);
// drop(client) joins the worker thread
```

Under CEF, spawn the worker from `AppBuilder::on_ready` (never before
`execute_process` — see [Native Workers](../workers/native-workers.md)).

## 3. Call it from JavaScript (web)

Build the wasm artifacts and serve them with COOP/COEP:

```sh
nix-shell shell.nix --run "cargo run -p xtask wasm"
nix-shell shell.nix --run "cargo run -p afterglow-web --example coep_server"
```

Then in a page — the `#[rpc]` macro generates a typed TS client, so you call
typed methods with no manual postcard encoding:

```ts
import { Rpc } from './rpc.js';
import { PhysicsClient } from './physics.client.js';

const transport = await Rpc.create({
  mainWasmUrl: 'afterglow_web.wasm',
  workerJsUrl: 'worker.js',
  workerWasmUrl: 'physics_worker.wasm',
});

const physics = new PhysicsClient(transport);
const result = await physics.step(new Float32Array([0, 1, 2]), 0.5);
// Float32Array [0.5, 1.5, 2.5]
transport.terminate();
```

Open <http://localhost:8787/worker-test.html> — the `worker-test.html` page does
exactly this and flips its title to `PASS`/`FAIL`.

## 4. The same service, both backends

The Rust `PhysicsWorker` impl and the `physics_worker.wasm` you just called from
JS are the **same** `#[rpc(worker = PhysicsWorker)]` service. The only thing
that differs between the two call sites is the transport underneath — an OS
thread with heap rings (native) versus a Web Worker with a
`SharedArrayBuffer` (web). The framing, the postcard encoding, and the
`Response` envelope are identical.

## Next

- [Building a Game Window](./game-window.md) — wire a worker into a CEF window.
- [Defining a Service](../workers/defining-a-service.md) — the full macro rules.
