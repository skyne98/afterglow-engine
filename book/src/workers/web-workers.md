# Web Workers

The web worker transport runs the same `#[rpc]` service as the native path, but
over a `SharedArrayBuffer` in a Web Worker instead of an OS thread. This is what
the CEF `minimal` example and the web target both use.

## Calling a worker from TypeScript

The `#[rpc(worker = ...)]` macro generates a typed TS client (see
[Defining a Service](./defining-a-service.md)). You construct it with the
low-level `Rpc` transport and call typed methods — no manual method IDs or
postcard encoding:

```ts
import { Rpc } from './rpc.js';
import { PhysicsClient } from './physics.client.js';

const transport = await Rpc.create({
  mainWasmUrl: 'afterglow_web.wasm',
  workerJsUrl: 'worker.js',
  workerWasmUrl: 'physics_worker.wasm',
  timeoutMs: 5000, // optional; default 5000
});

const physics = new PhysicsClient(transport);

// Typed — no manual postcard or method IDs.
const result = await physics.step(new Float32Array([0, 1, 2]), 0.5);
// Float32Array [0.5, 1.5, 2.5]

transport.terminate();
```

`Rpc.create(...)` instantiates the shared main wasm, spawns the worker, and
resolves once the worker reports `ready`. `timeoutMs` bounds both the init wait
and each call's response wait.

## The low-level `Rpc` transport

The generated TS client wraps `rpc.call(methodId, args)`; you only need the
raw `Rpc` to construct the transport. But for ad-hoc/raw calls or custom
codecs, the byte API is available:

```js
import { Rpc, concat, encodeF32Vec, encodeF32, decodeF32Vec } from './rpc.js';

const rpc = await Rpc.create({
  mainWasmUrl, workerJsUrl, workerWasmUrl, timeoutMs,
});

// Raw: manual method ID + postcard encoding.
const args = concat(encodeF32Vec([0, 1, 2]), encodeF32(0.5));
const result = decodeF32Vec(await rpc.call(0, args));   // Float32Array [0.5, 1.5, 2.5]
```

## The `Rpc` API

```js
const rpc = await Rpc.create({
  mainWasmUrl,   // page-side ring-buffer wasm
  workerJsUrl,   // the Web Worker script
  workerWasmUrl, // the service wasm
  timeoutMs,     // optional, default 5000
});

const resultBytes = await rpc.call(methodId, encodedArgs);  // Uint8Array
rpc.terminate();
```

### Call semantics

- **One in-flight call** (SPSC). A concurrent `call` rejects as `busy` without
  touching the rings.
- The request payload is `[method: u32 LE][postcard args]`.
- A successful `call` resolves to the inner response payload with the
  `Response` envelope removed. This `Uint8Array` **aliases the shared page
  scratch** — decode or copy it before starting the next call, or it will be
  overwritten. (Generated TS clients do this for you.)
- Server/decode envelopes reject with an `Error`.
- Every response wake is posted only *after* the response write index is
  published.

### Fatal lifecycle

The client latches the first fatal failure permanently into `_fatal`
(idempotent: the first failure wins) and never touches the rings again. Fatal
failures are: an init or response `timeoutMs` elapsing, a worker `onerror`, a
worker `error` message, or `terminate()`. Once latched, every later `call`
rejects immediately with the latched `Error`. Recover by constructing a new
`Rpc`; its `init_ring_buffers` resets both rings. See
[Lifecycle & Errors](./lifecycle.md).

## Postcard codecs

The postcard codec library lives in `codec.ts` — imported by both generated TS
clients and hand-written code. It provides typed encode/decode functions for
all supported types (primitives, `String`, `Vec<u8>`/`Vec<f32>`/`Vec<f64>`),
plus the low-level varint/zigzag primitives. See
[Defining a Service](./defining-a-service.md) for the full supported-types
table.

For types the macro doesn't support (custom structs, enums), use the raw
`rpc.call(methodId, args)` byte API with your own codecs on top.

## The files

`crates/afterglow-web/www/` contains:

| File | What it is |
|---|---|
| `afterglow_web.wasm` | The page-side ring buffer API (shared memory). |
| `physics_worker.wasm` | The demo `Physics` worker service. |
| `rpc.js` | The low-level `Rpc` transport (instantiate + `call`). |
| `codec.ts` | The postcard codec library (typed encode/decode for all supported types). |
| `physics.client.ts` | **Generated** typed TS client for `Physics`. |
| `worker.js` | The Web Worker that drives the service wasm. |
| `ring-buf.js` | Tested wrap-safe ring primitives (used by `worker.js`). |
| `worker-test.html` | A round-trip test page. |
| `worker-bench.html` | A benchmark page. |

Build them with `cargo run -p xtask wasm` (dev) or the optimized build in
[Web (Wasm)](../building/web.md). `xtask wasm` also copies generated `.ts`
clients into `www/`.

## Notes

- **The same service you ship natively.** `physics_worker.wasm` is the same
  `#[rpc(worker = PhysicsWorker)]` service that
  `PhysicsClient::spawn_worker(PhysicsWorker)` uses on the CEF path. Write it
  once; the framing and postcard encoding are identical.
- **One worker service per wasm cdylib.** The wasm exports use fixed
  `#[no_mangle]` names, so at most one `#[rpc(worker = ...)]` service per wasm
  module.
- **Events are native-only.** Worker→page notifications on web currently have to
  go through the response path.

## Next

- [Defining a Service](./defining-a-service.md) — the `#[rpc]` macro.
- [Lifecycle & Errors](./lifecycle.md) — timeouts, the poison/fatal latch,
  recovery.
- [Web (Wasm)](../building/web.md) — building the artifacts.
