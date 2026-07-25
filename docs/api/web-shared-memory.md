# `afterglow-rpc` wasm — shared-memory worker transport

> Status: working; API checked against the 2026-07-18 source.

## Architecture

The page owns a shared `WebAssembly.Memory` backed by `SharedArrayBuffer`. An
`afterglow-rpc` wasm instance exposes the page-side request/response ring API.
A Web Worker receives the SAB once during initialization and reads/writes those
rings with matching `Atomics` and wrap-safe framing logic.

The service wasm instance inside the Web Worker has **separate shared wasm
memory**. `worker.js` copies request arguments from the SAB into that instance's
input scratch, invokes the generated service export, then copies its encoded
response into the SAB response ring.

`postMessage` is wake-up only (`"wake"`); it never carries request, response, or
event payloads.

The page-side `AsyncWorker` uses 256 preallocated task slots indexed by
`task_id`, rather than a dynamically growing pending-call `Map`. Its browser
fetch imports use a separate 256-slot fixed table with bounded probing and slot
generation IDs. Calls fail with a deterministic capacity error before dispatch
if all slots are occupied. Generated wasm services reserve a matching 256-entry
completion queue and reject a 257th outstanding export call. One shared polling
pump remains active while the fixed pending count is nonzero and drains at most
32 completions per invocation.

## Source and deployment layout

- `web/src/engine/<subsystem>/` contains authored engine TypeScript.
- `web/src/workers/` contains worker runtime and generated typed clients.
- `web/src/demos/<name>/` contains pure game/presentation entrypoints.
- `web/public/`, `web/assets/`, and `web/contracts/` separate authored pages,
  deployable inputs, and development policy.
- `www/` is generated from those inputs by `scripts/build-web.ts`; it is never
  an authored source or package workspace and can be deleted and rebuilt.

The build replaces the complete deployment tree. `--check` compares every
staged file and rejects missing, stale, or extra output.

## Required page headers

`SharedArrayBuffer` requires a cross-origin-isolated page:

```http
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
Cross-Origin-Resource-Policy: same-origin
```

`crates/afterglow-web/examples/coep_server.rs` serves these headers for local
development. It is a bounded HTTP/1.1-close test server, not a public origin.
Path/MIME resolution and canonical confinement are shared with the web dev
server via [`afterglow-assets`](assets.md).

## Caddy HTTP/1.1, HTTP/2, and HTTP/3 origin

`deploy/web/Caddyfile` is the static-origin configuration. It explicitly
supports `h1 h2 h3`, leaves HTTP/1.1 persistent (it never sends
`Connection: close`), and applies the three isolation headers plus
`Accept-Ranges: bytes`. HTTP/2 and HTTP/3 multiplex all page ranges through one
connection; HTTP/3 uses QUIC and its negotiated idle timeout, so a
`Connection: keep-alive` header must not be emitted there.

For the single-client local gate, Caddy listens at `https://localhost:8443`
and uses its local CA without a privileged port-80 redirect listener:

```sh
nix-shell shell.nix --run \
  'caddy run --config deploy/web/Caddyfile --adapter caddyfile'
```

A public deployment sets `AFTERGLOW_WEB_ADDRESS` to its DNS hostname and must
expose TCP and UDP on its HTTPS port. It uses normal Caddy/ACME TLS; HTTP/3
falls back transparently to HTTP/2 or HTTP/1.1 when QUIC is unavailable. The
local CEF H3 benchmark uses a narrow certificate-SPKI allowlist and a forced
localhost QUIC origin because Chromium does not accept a custom local CA for
normal Alt-Svc-to-QUIC promotion. Those switches are test-only and are never a
production certificate policy.

## Memory contract

The page client creates:

```js
const memory = new WebAssembly.Memory({
  shared: true,
  initial: 256, // 16 MiB
  maximum: 1024 // 64 MiB
});
```

Both wasm modules must be linked with imported shared memory. Workspace
`.cargo/config.toml` supplies `--import-memory`, `--shared-memory`, a 64 MiB
maximum, and wasm atomic/bulk-memory features.

The page-side module currently contains:

- request ring: 1 MiB data + 12-byte header;
- response ring: 1 MiB data + 12-byte header;
- page scratch: 1 MiB.

Use exported pointers and sizes. Their offsets are linker-controlled and must
never be hard-coded.

## Page-side wasm exports (`afterglow_rpc.wasm`)

### Initialization and layout

```text
init_ring_buffers() -> void
get_request_ptr() -> usize
get_response_ptr() -> usize
get_buffer_size() -> usize   # total ring bytes: 12 + 1 MiB
get_scratch_ptr() -> usize
get_scratch_size() -> usize  # 1 MiB
```

Call `init_ring_buffers` once before starting the worker. It resets both rings.

### Transport operations

```text
write_frame(ptr, len) -> i32
read_response(ptr, max_len) -> i32
```

A successful `write_frame` publishes the request frame and calls the imported
`env.notify_worker` wake callback. The page only produces requests and consumes
responses; worker-side directions are implemented directly over the SAB in
`worker.js`, so benchmark-only reverse-direction exports are not exposed.

Read return codes:

| Value | Meaning |
|---:|---|
| `>= 0` | payload bytes copied |
| `-1` | ring empty |
| `-2` | output too small; frame remains queued |
| `-3` | corrupt ring/frame |

Write return codes are `0` success, `-1` full, and `-3` corrupt. Exported
pointer-taking functions require valid ranges in the module's linear memory.

## Generated service-wasm exports (`physics_worker.wasm`)

A `#[rpc(worker = WorkerType)]` service exports (see [`rpc-macro.md`](rpc-macro.md) for the macro):

```text
afterglow_wasm_init() -> void
afterglow_wasm_serve_frame(method, args_ptr, args_len, out_ptr, out_max) -> i32
afterglow_wasm_input_ptr() -> usize
afterglow_wasm_input_size() -> usize   # 1 MiB
afterglow_wasm_output_ptr() -> usize
afterglow_wasm_output_size() -> usize  # 1 MiB
```

`afterglow_wasm_init` constructs `WorkerType::default()`. Sync services use
`serve_frame`, which writes a postcard `Response` envelope. For generated async
services, `worker.js` detects `afterglow_wasm_serve_async`, drives
`afterglow_wasm_tick` inside the Web Worker, drains the task-ID completion, and
publishes only its response envelope to the SAB response ring. Thus CPU-heavy
methods such as Basis transcoding stay off the page thread while retaining the
same payload transport.

## JavaScript client (`www/rpc.js`)

```js
const rpc = await Rpc.create({
  mainWasmUrl: 'afterglow_rpc.wasm',
  workerJsUrl: 'worker.js',
  workerWasmUrl: 'physics_worker.wasm',
  timeoutMs: 5000, // optional; default 5000
});

const resultBytes = await rpc.call(methodId, encodedArgs);
rpc.terminate();
```

Current semantics:

- `Rpc.create({ mainWasmUrl, workerJsUrl, workerWasmUrl, timeoutMs })`
  instantiates the shared main wasm + spawns the worker and resolves once the
  worker reports `ready`. `timeoutMs` is optional and defaults to `5000`; it
  bounds both the init wait and each call's response wait.
- exactly one in-flight call (SPSC); a concurrent call rejects as `busy`
  without touching the rings;
- the request ring payload is `[method: u32 LE][postcard args]`;
- a successful `call` resolves to the inner response payload with the
  `Response` envelope removed; this `Uint8Array` aliases the shared page
  scratch, so decode or copy it before starting the next call;
- server/decode envelopes reject with an `Error`;
- every response wake is posted only after the response write index is
  published.

### Fatal lifecycle

The client latches the first fatal failure permanently into `_fatal`
(idempotent: the first failure wins) and never touches the rings again:

- Fatal failures are: the init or response `timeoutMs` elapsing, a worker
  `onerror`, a worker `error` message, or `terminate()`.
- Once latched, every later `call` rejects immediately with the latched
  `Error`, and late `ready` / response wakes are dropped — so a late reply to a
  timed-out call can never be consumed as a later call's result.
- A `create()` whose setup or init wait fails cleans up the Worker: it calls
  `rpc.terminate()` if the `Rpc` was constructed, otherwise
  `worker.terminate()` directly. The Worker is never leaked.
- `terminate()` latches a failure (idempotent if already latched) and stops
  the Worker exactly once via an internal `_terminated` guard, so it is safe to
  call from cleanup paths and from a failed `create`.
- Recover by constructing a new `Rpc`; its `init_ring_buffers` resets both
  rings.

`rpc.js` also exports the small codec helpers used by the demo:
`encodeVarint`, `decodeVarint`, `concat`, `encodeF32Vec`, `encodeF32`, and
`decodeF32Vec`. Decoders reject truncated and overflowing inputs. These helpers
and the pure wrap-safe operations in `www/ring-buf.js` are covered by the
no-dependency Node suite:

```sh
node --test crates/afterglow-web/tests/rpc.test.mjs
```

## Worker state and wake correctness (`www/worker.js`)

The worker is an ES module and imports the tested wrap-safe primitives from
`www/ring-buf.js`. It transitions `init → ready → running`. A wake arriving while it is
not awaiting is retained in `wakePending`, preventing lost notifications. When
the request ring is empty, the worker awaits the next wake and consumes no idle
CPU. Corrupt ring state is drained/resynchronized and reported to the page.

## Build

Development artifacts and deterministic copies into `www/`:

```sh
cargo run -p xtask wasm
```

Optimized artifacts for benchmarking/shipping:

```sh
cargo build -p afterglow-rpc --target wasm32-unknown-unknown \
  -Zbuild-std=core,alloc,std,panic_abort --profile wasm-release
cargo build -p afterglow-rpc-demo --target wasm32-unknown-unknown \
  -Zbuild-std=core,alloc,std,panic_abort --profile wasm-release
```

See [`ring-buffer.md`](ring-buffer.md) for framing/native APIs and
[`../research/performance-benchmarks.md`](../research/performance-benchmarks.md)
for current optimized results.
