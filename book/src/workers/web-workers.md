# Web Workers

The web worker transport runs the same `#[rpc]` service as the native path, but
over a `SharedArrayBuffer` in a Web Worker instead of an OS thread. This is what
the CEF `minimal` example and the web target both use.

## Calling a worker from TypeScript

The `#[rpc(worker = ...)]` macro generates a typed TS client (see
[Defining a Service](./defining-a-service.md)). You construct it with the
generated `spawn()` factory and call typed methods—no manual method IDs or
postcard encoding:

```ts
import { PhysicsClient } from './physics.client.ts';

const physics = await PhysicsClient.spawn({
  workerWasmUrl: 'physics_worker.wasm',
  timeoutMs: 5000,
});

// Typed — no manual postcard or method IDs.
const result = await physics.step(new Float32Array([0, 1, 2]), 0.5);
// Float32Array [0.5, 1.5, 2.5]

physics.close();
```

`PhysicsClient.spawn(...)` instantiates the shared main wasm, spawns the worker,
and resolves once it reports `ready`. `close()` is idempotent. `timeoutMs`
bounds both the init wait and each call's response wait.

## The low-level `Rpc` transport

The generated TS client wraps `rpc.call(methodId, args)`. Only transport
protocol diagnostics and custom codecs should use the raw byte API:

```js
import { Rpc, concat, encodeF32Vec, encodeF32, decodeF32Vec } from './rpc.js';

const rpc = await Rpc.create({
  mainWasmUrl, workerJsUrl, workerWasmUrl, timeoutMs,
});

// Raw: manual method ID + postcard encoding.
const args = concat(encodeF32Vec([0, 1, 2]), encodeF32(0.5));
const result = decodeF32Vec(await rpc.call(0, args));   // Float32Array [0.5, 1.5, 2.5]
```

## Bounded async tasks

`AsyncWorker` has 256 preallocated task slots keyed by `task_id` and a separate
256-slot browser-fetch table with bounded probing. Exhaustion is reported before
dispatch instead of growing pending maps. The wasm service reserves 256
completion entries during initialization, while each JS `poll()` drains at most
32. All task slots share one polling pump, so call count does not create one
timer loop per RPC or an unbounded per-frame completion burst.

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

Authored worker runtime and generated typed clients live together under
`crates/afterglow-web/web/src/workers/`. Wasm inputs are staged under
`web/assets/`; diagnostic pages are authored under `web/public/` and
`web/src/demos/`.

`cargo run -p xtask wasm` refreshes those generated inputs and rebuilds
`crates/afterglow-web/www/`. The `www/` tree contains deployment `.js`, `.wasm`,
HTML, and cooked assets only. It contains no TypeScript, tests, package-manager
state, manifests, or vendored Three.js source and may be deleted and rebuilt at
any time.

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
