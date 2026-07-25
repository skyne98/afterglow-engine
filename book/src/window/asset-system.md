# The Asset System

> **Stale.** `afterglow-cef` and its CEF scheme handler / native range bridge
> have been removed. The CEF-specific prose below is retained as design context;
> the native shell's asset-root loader is an open parity gate (G1) tracked in
> `docs/implementation/shell-promotion-plan.md`. Treat references to the CEF
> scheme, the native bridge, and `AppBuilder::on_ready` as historical. The
> authoritative current state is `docs/api/asset-system.md`.

afterglow-engine has a streaming, range-capable asset system that works on
both backends. Assets are served from the filesystem (native) or over HTTP
(web), with no whole-file buffering and partial reads at arbitrary offsets.

## Current layers

1. **Serving layer** — the live browser BIG/VT path. Public web uses HTTP Fetch
   + Range. CEF uses scheme Fetch for singleton reads and a private bounded
   process-message bridge for bulk ranges. Both ultimately stream from
   `AssetSource::read_at`.
2. **Asset loader worker API** — a generated async service available to native
   Rust consumers. It is not currently the entry point used by
   `BigAssetSession`; the two browser targets do not currently share one
   `AssetLoaderClient` path.
3. **Texture transcode service** — public web uses generated WASM Web Workers.
   CEF currently does the same, but this is a known target-boundary defect:
   `afterglow-texture` has a native implementation and must run through its
   generated native client and an OS worker started from `AppBuilder::on_ready`.

## `AssetSource` — the streaming primitive

```rust
pub trait AssetSource {
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
container. It requires explicit worker count, transcode waiting capacity,
VT pending page/byte capacities, urgent/quality batch deadlines, and maximum
header bytes; validates them before spawning its standard
typed transcoder workers; owns
the raw loader and VT page provider; and rolls partial startup back in reverse
order. `createAssetStore()` starts the engine-owned mesh optimizer and binds
fixed asset/completion capacities to that raw loader. It creates at most one
`VirtualTextureStore`. Idempotent shutdown closes
all workers even if one close fails, with stable error telemetry. Passing
`telemetry: runtime.telemetry` correlates session startup, range work, feedback,
scheduler and bulk waits, native RPC round trips, transcode, mesh optimization,
and VT publication. Persistent derived-page caching has been removed; every
nonresident page follows source read and transcode. Pages do not construct a raw `Rpc` transport, choose worker
scripts, or manually terminate workers.

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

CEF bulk asset reads bypass `fetch` through an internal native bridge. The
browser process merges spans that are already adjacent in supplied order and
transfers one shared-memory response to the renderer. A bounded renderer-local
thread performs the V8 sandbox's one required copy; returned page arrays are
zero-copy views over the resulting buffer. An explicitly source-sorted
fox-laptop diagnostic measured **950.2 MiB/s median**, about 2.15× the prior
441.2 MiB/s custom-scheme/fetch result, with no drops at its intended batch
cadences. The live `BigAssetSession` provider does not yet wire that source-
sorting helper, so this is not current gameplay-provider throughput.

The bridge and web transport share hard bounds: 256 spans, 4 MiB per complete
response, two responses, and 8 MiB total in flight. The live provider currently
preserves scheduler/admission order; only the separate diagnostic/helper source-
sorts. The bridge merges adjacent spans only when they arrive adjacent. Single
header/model reads
remain on the scheme/HTTP path, so they cannot race the bridge's two global bulk
slots. Capacity exhaustion rejects
immediately instead of growing a queue. The bridge is CEF-only; public web uses
explicit non-overlapping HTTP multi-ranges and receives standard `206
multipart/byteranges` responses. The CEF scheme and development server can
still serve those standard responses, while Caddy supplies the same format
from its static-file implementation. Ordinary single ranges (`bytes=0-499`,
`bytes=500-`, and `bytes=-500`) remain supported.

In-flight HTTP and CEF bulk reads currently have no response deadline and do not
receive VT abort signals; cancellation takes effect at stage boundaries. The
Rust development server also reads request headers only once into 8 KiB and
closes every connection, so use it only locally. `deploy/web/Caddyfile` is the
production web-serving profile.

## Where the code lives

Asset serving code is split across 5 places, each with a distinct role:

| Where | Role |
|---|---|
| `afterglow-assets/src/` (`lib.rs`, `source.rs`, `range.rs`) | **The shared core.** `AssetSource` trait, `FsSource`/`BytesSource`, `parse_range`, `AssetRoot`/`resolve` (confinement), `guess_mime`. Both backends build on this — no backend-specific logic here. |
| `afterglow-cef/src/resources.rs` | **CEF scheme adapter.** Resolves a path to an `AssetSource`, drives CEF's `ResourceHandler` (`open`/`skip`/`read`/`response_headers`). Sets COOP/COEP + `Accept-Ranges`. |
| `afterglow-web/src/dev_server.rs` | **HTTP adapter + bounded server.** Parses `Range` → `206`/`Content-Range`, streams via `stream_body()`, and runs fixed workers with bounded per-worker queues through `DevAssetServer`. |
| `afterglow-assets-worker/src/` (`lib.rs`, `fetch.rs`) | **Generated asset-loader service.** Native builds read through `FsSource`; a web fetch ABI exists, but the live browser BIG/VT path currently bypasses this service and uses the serving-layer range loader. |
| `afterglow-web/web/src/workers/async-worker.ts` | **Web async worker driver.** Authored TypeScript that drives the wasm async worker executor (`tick` + bounded `drain_completion`) and provides the `ag_fetch_start`/`ag_fetch_poll` imports. |

**The principle:** the *what* (streaming reads, ranges, confinement, MIME) lives
in `afterglow-assets`; the *how to deliver it* (CEF vs HTTP vs RPC) lives in
each adapter. No streaming/range logic is duplicated across adapters.

## The asset loader worker API

The `afterglow-assets-worker` crate provides an `#[rpc]` async singleton with
full-load, size, and positional-read methods:

```rust
#[rpc(worker = AssetLoaderWorker, singleton)]
pub trait AssetLoader {
    async fn load(path: String) -> RpcResult<Vec<u8>>;
    async fn size(path: String) -> RpcResult<u64>;
    async fn read(path: String, offset: u64, len: u32) -> RpcResult<Vec<u8>>;
}
```

The macro generates both a native Rust client and a TypeScript client. They are
target-specific backends, not permission to run the WASM client in CEF. The
`singleton` flag means native `spawn_worker()` takes no arguments (constructs
via `Default`); the asset root is set once via `set_asset_root`.

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

### Generated TypeScript client

A generated `AssetLoaderClient.spawn()` can instantiate the WASM service and
its Fetch imports. This is a public-web-capable API, but it is not the path used
by `BigAssetSession` today and it is not a permitted CEF replacement for the
native service:

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

On public web, these fetches hit the development server or production origin.
On CEF, do **not** instantiate `assetloader.wasm` merely because the page can
fetch it. A composed CEF asset-loader service must be spawned natively from
`AppBuilder::on_ready`. The live BIG tile path currently uses the generic CEF
range bridge directly, while its texture transcode stage incorrectly remains a
WASM Web Worker; both facts are tracked as implementation status rather than
presented as target policy.

The poll model is documented in
[Defining a Service](../workers/defining-a-service.md).

## Next

- [Defining a Service](../workers/defining-a-service.md) — the `#[rpc]` macro,
  `singleton` flag, and the poll model.
- [Native Workers](../workers/native-workers.md) — sync + singleton worker transport.
- [Lifecycle & Errors](../workers/lifecycle.md) — timeouts, poison, thread safety.
