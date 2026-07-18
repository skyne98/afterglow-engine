# The Asset System

afterglow-engine has a streaming, range-capable asset system that works on
both backends. Assets are served from the filesystem (native) or over HTTP
(web), with no whole-file buffering and partial reads at arbitrary offsets.

## Two layers

1. **Serving layer** — browser `fetch` hits this. The CEF scheme handler
   (native) and the web HTTP dev server both stream via
   `AssetSource::read_at` and support `Range` requests.
2. **Asset loader worker** — the **single portable entry point** for loading
   assets from the render thread (and Rust subsystems). Works on both native
   (`FsSource`/`pread`) and web (JS-imported `fetch`). The render thread uses
   the generated `AssetLoaderClient` TS client on both backends.

## `AssetSource` — the streaming primitive

```rust
pub trait AssetSource: Send + Sync {
    fn len(&self) -> u64;
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize>;  // 0 at EOF
    fn etag(&self) -> Option<String>;
}
```

Positional reads — no shared cursor, so concurrent range requests share one
open `File` via `pread`. Two impls:

- **`FsSource`** (native) — a file on disk, read via `pread` (no mutex, no
  whole-file load). The asset worker retains a fixed round-robin cache of 16
  open sources for repeated container ranges.
- **`BytesSource`** — an in-memory `&'static [u8]` (the one embedded asset,
  `index.html`).

## Fixed engine asset states

`AssetStore` defaults to 1,024 registered assets. Register manifest paths during
bootstrap to obtain numeric `AssetId`s, then use `tryLoadAsset`. Reads and parses
advance through fixed numeric state tables. Completed promises enter a
preallocated ring, and `poll()` publishes at most 32 per frame by default.
Capacity exhaustion is explicit rather than growing an engine queue; stale or
evicted generations cannot publish later.

The path-based `load()` API remains a game-facing convenience wrapper. It
registers the path and throws if the configured asset capacity is exhausted.

`BigAssetSession.open()` is the browser bootstrap owner for one `.big`
container. It requires explicit worker count, transcode-queue capacity, and
maximum header bytes; validates the header bound before spawning its standard
typed transcoder workers; owns
the raw loader and VT page provider; and rolls partial startup back in reverse
order. `createAssetStore()` starts the engine-owned mesh optimizer and binds
fixed asset/completion capacities to that raw loader. It creates at most one
`VirtualTextureStore`. Idempotent shutdown closes
all workers even if one close fails, with stable error telemetry. Pages do not
construct a raw `Rpc` transport, choose worker scripts, or manually terminate
workers.

Self-contained GLBs follow this same path. The offline pipeline extracts
embedded images into virtual textures, removes their payload buffer views, and
compacts the runtime GLB. An ignored metadata extension retains image indices,
UV channels, KHR texture transforms, and samplers. The image-free model remains
a seekable raw `.big` asset. `BigContainerAssetLoader` exposes packed model
chunks through the regular loader contract. `AssetStore.loadOptimizedGLTF()` preserves the full
scene, skeleton, skin attributes, morph targets, and animations while the
runtime meshopt worker reorders triangle indices for vertex-cache and overdraw
efficiency. Material groups are processed independently. Skinned meshes are not
simplified yet because a position/UV-only error metric cannot safely evaluate
animated bone deformation.

## Range support

Both serving backends parse the HTTP `Range` header (single-range only):
`bytes=0-499`, `bytes=500-`, `bytes=-500`. A range request produces `206
Partial Content` with `Content-Range`; no range → `200` full (also streamed).
Multi-range falls back to `200` full.

## Where the code lives

Asset serving code is split across 5 places, each with a distinct role:

| Where | Role |
|---|---|
| `afterglow-assets/src/` (`lib.rs`, `source.rs`, `range.rs`) | **The shared core.** `AssetSource` trait, `FsSource`/`BytesSource`, `parse_range`, `AssetRoot`/`resolve` (confinement), `guess_mime`. Both backends build on this — no backend-specific logic here. |
| `afterglow-cef/src/resources.rs` | **CEF scheme adapter.** Resolves a path to an `AssetSource`, drives CEF's `ResourceHandler` (`open`/`skip`/`read`/`response_headers`). Sets COOP/COEP + `Accept-Ranges`. |
| `afterglow-web/src/dev_server.rs` | **HTTP adapter + bounded server.** Parses `Range` → `206`/`Content-Range`, streams via `stream_body()`, and runs fixed workers with bounded per-worker queues through `DevAssetServer`. |
| `afterglow-assets-worker/src/` (`lib.rs`, `fetch.rs`) | **The portable asset loader.** `#[rpc]` async worker that works on both backends. Native: `FsSource`/`pread`. Web: JS-imported `fetch` (the `fetch.rs` bridge). The render thread uses the generated `AssetLoaderClient` TS client on both. |
| `afterglow-web/web/src/workers/async-worker.ts` | **Web async worker driver.** Authored TypeScript that drives the wasm async worker executor (`tick` + bounded `drain_completion`) and provides the `ag_fetch_start`/`ag_fetch_poll` imports. |

**The principle:** the *what* (streaming reads, ranges, confinement, MIME) lives
in `afterglow-assets`; the *how to deliver it* (CEF vs HTTP vs RPC) lives in
each adapter. No streaming/range logic is duplicated across adapters.

## The asset loader worker (both backends)

The `afterglow-assets-worker` crate is the **single portable entry point**
for asset loading. It's an `#[rpc]` async singleton worker that works on both
backends:

```rust
#[rpc(worker = AssetLoaderWorker, singleton)]
pub trait AssetLoader {
    async fn load(path: String) -> RpcResult<Vec<u8>>;
    async fn size(path: String) -> RpcResult<u64>;
    async fn read(path: String, offset: u64, len: u32) -> RpcResult<Vec<u8>>;
}
```

The render thread uses the generated `AssetLoaderClient` (TS) — the same client
on both backends. The `singleton` flag means `spawn_worker()` takes no arguments
(constructs via `Default`); the asset root is set once via `set_asset_root`.

```rust
// Set the root once (e.g. from AppBuilder::on_ready).
AssetLoaderWorker::set_asset_root(AssetRoot::new("assets")?);

// Spawn the singleton worker — returns the client directly.
let client = AssetLoaderClient::spawn_worker()?;

// Full load (small files).
let fut = client.load("textures/sky.png".into())?;
client.poll();
let bytes: Vec<u8> = fut.await?;

// Streaming (large files): size + read in chunks.
let size: u64 = client.size("models/world.gltf".into())?.await?;  // (after poll)
let chunk = client.read("models/world.gltf".into(), 1024, 4096)?;  // 4 KiB at offset 1 KiB
client.poll();
let bytes: Vec<u8> = chunk.await?;
```

See [Defining a Service](../workers/defining-a-service.md) for the `singleton` flag
and the async-vs-sync comparison table.

### Using it from TypeScript (render thread)

The generated `AssetLoaderClient` has a static `spawn()` that does all the
wasm instantiation, memory setup, and fetch-import wiring internally — you get
back a ready-to-use client:

```ts
import { AssetLoaderClient } from './assetloader.client.js';

// One call — instantiates the worker, wires everything, returns the client.
const loader = await AssetLoaderClient.spawn();

// Full load (small files).
const bytes = await loader.load('textures/sky.png');

// Streaming (large files): size + read in chunks.
const size: number = await loader.size('models/world.gltf');
let offset = 0;
while (offset < size) {
  const chunk = await loader.read('models/world.gltf', offset, 65536); // 64 KiB
  // ... process chunk ...
  offset += chunk.length;
}

// Each frame: poll to drive the executor + resolve pending promises.
function frame() {
  loader.poll();
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);
```

On native (CEF), the same code runs — `fetch('asset_loader.wasm')` hits the
`afterglow://` scheme, and `fetch('textures/sky.png')` inside the worker hits
the same scheme (streamed via `FsSource`/`pread`). On web, both hit the HTTP
dev server or your production origin. The code is identical.

The poll model (how `await` resolves under the hood) is documented in
[Defining a Service](../workers/defining-a-service.md).

## Next

- [Defining a Service](../workers/defining-a-service.md) — the `#[rpc]` macro,
  `singleton` flag, and the poll model.
- [Native Workers](../workers/native-workers.md) — sync + singleton worker transport.
- [Lifecycle & Errors](../workers/lifecycle.md) — timeouts, poison, thread safety.
