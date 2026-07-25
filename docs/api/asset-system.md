# `afterglow-assets` + `afterglow-assets-worker` API — streaming assets + async loader

> Status: working with one audited integration gap; API checked against the
> 2026-07-22 source. The live BIG provider does not yet source-sort bulk spans.
> `afterglow-cef` has been removed; the native target boundary and native
> worker composition are tracked in `docs/implementation/shell-promotion-plan.md`.

## Browser BIG asset session

`afterglow-web/web/src/engine/assets/big-asset-session.ts` provides `BigAssetSession`, the
bootstrap owner for one seekable `.big` source. `open()` requires explicit
`workerCount`, `transcodeQueueCapacity`, `maxHeaderBytes`, and target GPU
format. Engine-owned typed transcoders are the default; tests/platform adapters
may inject a factory. It validates the 16-byte prefix and configured header bound
before starting workers, parses the header once, constructs the direct raw-asset
loader and VT page provider, and rolls workers back in reverse order after any
startup failure.

The public browser barrel is `web/src/engine/assets/index.ts`. The policy-free
`readBigHeader(source, path, maxHeaderBytes)` primitive performs the shared
bounded prefix/header read used by both VT/model archives and static meshes; no
consumer duplicates container validation. The session currently creates typed transcoder clients through
`spawnThreaded()` and closes every client in reverse order. Games never
construct `Rpc`, select worker scripts, or terminate raw transports.
`createTranscoder` is the platform/test injection boundary.

**Native target gap:** `afterglow-cef` has been removed. The default
transcoder factory is not target-aware, and the native shell does not yet
compose the generated native `afterglow-texture` client as an OS worker.
`afterglow-texture` has a generated native client and therefore must run as an
OS worker started from the shell's native bootstrap; until that lands, there is
no native host that wires native texture workers. Documentation must not treat
a WASM-on-native path as a supported backend.
`createAssetStore(capacity, completionsPerPoll)` starts and owns the standard
mesh optimizer and binds a fixed-capacity
`AssetStore` to the session's raw loader, so demos do not reconstruct container
ownership. A session creates at most one `VirtualTextureStore`; a second request or a
request after shutdown fails deterministically. Only one session-owned
`AssetStore` is allowed. `close()` first disposes its asset and VT stores, is
idempotent, closes all workers in reverse order even when one close fails, and reports stable
started/close-error/closed telemetry. The optional persistent blob cache remains
a generic caller-supplied byte store. The session applies the configured
transcode queue capacity instead of an implicit provider default.

```ts
const session = await BigAssetSession.open({
  containerPath: 'world.big',
  format,
  workerCount: 4,
  transcodeQueueCapacity: 64,
  maxHeaderBytes: 2 * 1024 * 1024,
  cache,
});
const assets = await session.createAssetStore(64, 8);
const store = session.createVirtualTextureStore(device, tuning);
```

`createFetchRangeLoader()` also exposes `readBulk(path, ranges)`. Public web
uses standard multipart HTTP ranges for both singleton and bulk reads; the
former CEF native shared-message bridge has been removed. The live session
page provider preserves scheduler/admission order. It does **not** currently
source-sort spans. `createPageRangeReader()` contains a separate source-sorting
implementation and restores caller order by page index, but `BigAssetSession`
does not call it. The session's page provider owns a fixed 256-slot
two-tier queue: mip+2 parent misses open a non-resettable 1 ms maximum window;
exact pages open a non-resettable 100 ms window. Each response is at most 4 MiB
and at most two responses / 8 MiB are in flight. `close()` rejects queued work
and prevents late raw bytes from entering a closed transcoder pool. Stable
telemetry reports queue depth, in-flight response bytes, urgent/quality batches,
rejects, and cancellations.

## Streaming sources (`afterglow-assets::source`)

```rust
pub trait AssetSource {
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool { self.len() == 0 }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize>;  // 0 at EOF
    fn etag(&self) -> Option<String> { None }
}
```

Positional, streaming reads — no shared cursor, so concurrent range requests
share one open `File` via `pread`. Both serving backends and the asset loader
worker build on this trait.

### `FsSource` (native only, `cfg(not(target_arch = "wasm32"))`)

```rust
pub struct FsSource { /* File + size + mtime etag */ }
impl FsSource {
    pub fn open(path: impl AsRef<Path>) -> Option<Self>;
    pub fn from_file(file: File, size: u64, etag: Option<String>) -> Self;
}
```

The current native implementation uses Unix `FileExt::read_at` (`pread`) — no
mutex and no whole-file load. ETag is derived from modified-time (weak,
mtime-based).

### `BytesSource`

```rust
pub struct BytesSource(pub &'static [u8]);
```

In-memory byte source for the one embedded asset (`index.html`). Works on all
targets.

### `AssetRoot::open_source`

```rust
impl AssetRoot {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_source(&self, url_path: &str) -> Option<FsSource>;
}
```

Resolve-then-open: delegates to `resolve` (canonically confined) then opens
an `FsSource`. Returns `None` if missing/escaped/unreadable.

### `AssetSourceCache` (native only)

```rust
pub const DEFAULT_SOURCE_CACHE_CAPACITY: usize = 16;
pub struct AssetSourceCache { /* fixed open-source slots */ }
impl AssetSourceCache {
    pub fn new(root: AssetRoot) -> Self; // fixed 16 slots
    pub fn with_capacity(root: AssetRoot, capacity: usize) -> Self;
    pub fn invalidate(&self, url_path: &str);
    pub fn clear(&self);
}
```

The cache retains `pread`-safe source handles with fixed capacity and
round-robin replacement. A hit performs no canonicalization, `File::open`, or
metadata call; it only clones the retained `Arc<FsSource>`. Capacity must be
positive. A retained handle intentionally continues to read its old inode if an
asset is replaced; producers must call `invalidate(path)` or `clear()` after a
rebuild. The web dev server constructs one default-16 cache shared by its
request handlers and retains the `.big` descriptor across requests.

### Native range bridge (removed)

The CEF shared-message bridge (`readCefNativeRanges` /
`afterglowNativeReadRanges`) and the `afterglow://local/` scheme handler in
`afterglow-cef/src/resources.rs` have been removed with `afterglow-cef`. The
web target's `createFetchRangeLoader` now routes both singleton and bulk reads
through standards-based HTTP Fetch + Range. A native equivalent for the
`afterglow-shell` host (asset-root loader + optional native range read) is an
open parity gate — see `docs/implementation/shell-promotion-plan.md`.

## Bounded bulk ranges (`afterglow-assets::multipart`)

```rust
pub const MAX_BULK_RANGES: usize = 256;
pub const MAX_BULK_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;
pub fn parse_multi_range(header: Option<&str>, total: u64) -> MultiRangeSpec;
pub struct MultipartSource { /* bounded envelope + streaming source */ }
```

Authored clients send explicit non-overlapping `bytes=START-END,...` spans.
`MultipartSource` emits standard `multipart/byteranges` with a stable boundary
and delegates payload reads to the underlying `AssetSource`; it never assembles
the payload in a temporary response buffer. Range count and complete response
bytes are hard-capped. Caddy provides the same standard response directly from
its static file server.

## Range parsing (`afterglow-assets::range`)

```rust
pub enum RangeSpec {
    Range { start: u64, end: u64 },  // bytes=START-END (inclusive, clamped)
    Full,                              // no range / unparseable → serve full 200
    Unsatisfiable,                     // out of range → 416
}

pub fn parse_range(header: Option<&str>, len: u64) -> RangeSpec;
pub fn content_range(start: u64, end: u64, total: u64) -> String;
```

`parse_range` is single-range only (`bytes=0-499`, `500-`, `-500`) and remains
whitespace-tolerant. Serving adapters call `parse_multi_range` first, then use
this parser for ordinary requests.

## Serving layer

### Native shell asset loader (pending)

The `afterglow-shell` host does not yet expose an equivalent asset-root loader;
serving on the native target is an open parity gate tracked in
`docs/implementation/shell-promotion-plan.md`. The previous CEF scheme handler
(`afterglow-cef/src/resources.rs`) has been removed.

### Web HTTP dev server (`afterglow-web/src/dev_server.rs`)

- `handle_request(root, request) -> Response` — parses `Range` header.
- Single `Range` → `206 Partial Content` + `Content-Range`; multi-range → one
  bounded `206 multipart/byteranges`; no range → `200` full.
- `stream_body(resp, writer, chunk)` — streams via `read_at` in chunks.
- `Accept-Ranges: bytes` on every response (via `CROSS_ORIGIN_HEADERS`).
- `DevAssetServer::start(root, address, workers, queue_per_worker)` owns a fixed
  worker set and fixed synchronous queues. Full queues reject connections
  deterministically; no connection creates a thread.
- `stats()` returns stable accepted/rejected/completed counters. `shutdown()`
  and `Drop` are idempotent and join the accept thread and workers.
- `cargo run -p xtask -- serve` runs four workers with sixteen queued
  connections per worker and handles Ctrl-C through the shutdown token.
- The current development server performs one `TcpStream::read` into an 8 KiB
  request buffer and always closes the connection. TCP does not guarantee that
  all headers arrive in that read; a fragmented `Range` header can therefore be
  missed. Treat this server as a local development tool, not the production web
  origin. `deploy/web/Caddyfile` is the production static-serving profile.

Neither HTTP bulk Fetch currently has a response deadline or
transport-level abort. VT cancellation is checked before dispatch and after
completion, but an in-flight stalled response can retain one of the fixed slots
indefinitely.

## Asset-loader worker versus the live BIG path

`afterglow-assets-worker` provides generated native and TypeScript clients, but
it is **not** the entry point used by `BigAssetSession` today. The live BIG/VT
path uses `createFetchRangeLoader`: public web issues HTTP Fetch + Range for
both singleton and bulk reads (the former CEF native bridge has been removed).
`afterglow-assets-worker` remains available to Rust/native consumers and
retains up to 16 open `FsSource` handles, but claims that every render-thread
asset load passes through `AssetLoaderClient` are incorrect.

Target policy remains unchanged: when this service is composed into the native
shell, it must use its generated native client and OS worker, not
`assetloader.wasm`. Public web may use the serving-layer Fetch path.

## Engine `AssetStore`

`AssetStore` has a fixed capacity (1,024 by default). `registerAsset(path)`
interns a path during manifest/bootstrap and returns a numeric `AssetId`; it
returns `-1` rather than growing past capacity. `tryLoadAsset(id, parser)` uses
direct `Uint8Array` state and object-handle tables. The game-facing `load(path,
parser)` wrapper performs registration and throws on capacity exhaustion.

## Resident (non-virtual) textures

Resident textures are single-mip, always-resident byte streams stored as
`AssetType::Texture` chunks in a `.big` container, sampled directly at runtime
— no page table, no mip tail, no VT feedback. The canonical use is the POM
height field (8-bit R8), kept out of VT so the march loop pays one direct
mip-0 fetch per step; normals/albedo/masks remain VT-streamed.

### `.big` v6 container format

v6 adds an explicit `TextureFormat` (`Rgba8` | `R8`) to `ChunkMeta::Texture`.
v5 files remain readable (they never contain `Texture` chunks, so the
`ChunkMeta::Texture` encoding carrying a `format` field is unambiguous); the
parser accepts `[5, 6]`, the writer writes 6. Resident texture chunks are
uncompressed (`Compression::None`) so loading has no meshopt-worker dependency.

### Cook

```
afterglow-pipeline resident-texture <input.r16|png> [<input2>...] <output.big> [--format r8|rgba8] [--name <name>]
afterglow-pipeline blue-noise <size> <output.big> [--name <name>]
```

`resident-texture` accepts multiple inputs (each becomes a named asset in one
container). `.r16` inputs are decoded losslessly and quantized 16→8 via
`(sample + 128) / 257` (deliberate cook-time quantization, not browser
truncation). `blue-noise` generates a tileable void-and-cluster dither tile.

### Runtime

`engine/assets/resident-texture.ts`:
- `findResidentTextureChunk(header, name)` — validates the chunk shape/format.
- `loadResidentTexture(three, source, header, name)` — reads + builds a
  `DataTexture` (`R8`→`r8unorm`, `Rgba8`→RGBA), repeat-wrapped, no-mipmap,
  `flipY=false`. Caller owns disposal.

Loading completes before renderer sealing. See `docs/api/pom.md` for the POM
height + blue-noise dither wiring.

States are `Free → Idle → Reading → Parsing → ReadyToPublish → Ready`, with an
`Error` terminal/retry state. Promise callbacks only place `(id, token, kind,
value)` into a preallocated completion ring. `poll()` publishes at most 32
completions by default, so microtask timing cannot mutate visible asset state in
the middle of rendering. Generation tokens reject evicted/stale reads and
dispose stale parsed assets. Queue depth, high-water, and overflow counters are
incremental. `cachedPaths` remains an explicit allocating diagnostic snapshot.

### Packed GLB and runtime mesh optimization

`afterglow-pipeline process` treats a self-contained `.glb` as an ordinary model
asset. It extracts every embedded image into a separately paged UASTC virtual
texture named `<model>#image-N`, then removes those browser-decodable payloads
from the runtime GLB. Material texture infos, UV sets, KHR transforms, and
samplers move into the ignored `AFTERGLOW_virtual_textures` JSON extension;
material factors remain in the standard material. Image buffer views are removed,
remaining references are remapped, and the BIN chunk is compacted. Occlusion,
unsupported material extensions, textured transmission, or non-base virtual
channels that the current binding cannot reproduce fail the cook rather than
silently losing shading. Factor-only `KHR_materials_transmission` is retained.
The resulting
image-free GLB is stored as one uncompressed, seekable `Mesh/Raw` chunk—arbitrary
container bytes are never passed through meshopt's vertex-stream codec. For
`.gltf` packages, the cook
confines every external buffer/image URI to the source directory and embeds the
side files into a self-contained GLB before packing; traversal and unsupported
image types fail closed.

`BigContainerAssetLoader(rangeLoader, containerPath, header)` indexes raw chunks
once and implements the normal `AssetLoader` `load/size/read` contract with exact
container range requests. `AssetStore.loadOptimizedGLTF(path, gltfLoader)` then:

1. loads the GLB through that ordinary asset interface;
2. parses the complete scene, skin, skeleton, morph targets, and animations;
3. sends each triangle group's indices through the meshopt worker's vertex-cache
   and overdraw passes;
4. replaces only the index buffer and publishes `meshOptimization` telemetry;
5. publishes `materialTextures` from standard or
   `AFTERGLOW_virtual_textures` metadata, mapping each glTF material's roles,
   image indices, UV channels, transforms, and sampler state without decoding
   an imported browser image.

Vertex identity never changes, so `JOINTS_0`, `WEIGHTS_0`, normals, tangents,
UVs, arbitrary attributes, morph targets, bind matrices, and animation tracks
remain attached to the same vertices. Material-group ranges are optimized
independently. Runtime LOD simplification is intentionally disabled for skinned
scenes until the worker's simplification error metric includes skin weights;
position/UV-only simplification would be visually unsafe during deformation.

## Asset loader worker (`afterglow-assets-worker`)

```rust
#[rpc(worker = AssetLoaderWorker, singleton)]
pub trait AssetLoader {
    async fn load(path: String) -> RpcResult<Vec<u8>>;
    async fn size(path: String) -> RpcResult<u64>;
    async fn read(path: String, offset: u64, len: u32) -> RpcResult<Vec<u8>>;
}

pub struct AssetLoaderWorker { /* singleton root + 16-source native cache */ }
impl AssetLoaderWorker {
    pub fn set_asset_root(root: AssetRoot);
}
```

Async `#[rpc]` service. `load` reads a file via `FsSource::read_at` and returns
its bytes (postcard-encoded `Vec<u8>`). The client uses the poll model:

```rust
AssetLoaderWorker::set_asset_root(AssetRoot::new("assets")?);
let client = AssetLoaderClient::spawn_worker()?;
let fut = client.load("path".into())?;  // non-blocking, returns a Future
client.poll();  // each frame: drain completions, resolve futures
let bytes: Vec<u8> = fut.await?;
```

## Async `#[rpc]` transport (`afterglow-rpc::native`)

### Framing

- Request: `[method: u32 LE][task_id: u64 LE][postcard args]`
- Completion: `[task_id: u64 LE][Response envelope]` on the response ring

### `AsyncWorkerTransport`

```rust
pub struct AsyncWorkerTransport { /* req, resp, handle, pending, next_task_id */ }
impl AsyncWorkerTransport {
    pub fn call_async(&self, method: u32, args: &[u8]) -> RpcResult<Oneshot>;
    pub fn poll(&self);
}
```

- `call_async` mints a `task_id`, writes the request, registers a pending
  `Oneshot` future, returns it. Never blocks.
- `poll` drains the response ring, matches `task_id`s, resolves `Oneshot`s.

### `Oneshot`

```rust
pub struct Oneshot { /* Arc<OneshotInner> */ }
impl Future for Oneshot {
    type Output = RpcResult<Vec<u8>>;
}
```

A poll-based future — resolves when `poll()` delivers the completion. No waker
registration needed (the caller's executor polls it; `poll()` is the driver).

### `spawn_async_worker_loop`

```rust
pub fn spawn_async_worker_loop<S, F>(
    impl_: S, capacity: usize, serve_async: F,
) -> RpcResult<(AsyncWorkerTransport, EventReceiver)>
where
    S: Send + 'static,
    F: Fn(&S, u32, &[u8]) -> ServeFuture + Send + 'static;
```

Spawns a worker thread with an `async-executor::LocalExecutor`. The loop:
1. Drains request frames.
2. Spawns `serve_async(impl, method, args)` on the executor.
3. When a task completes: writes `[task_id][Response]` to the response ring,
   unparks the client.
4. Ticks the executor (`executor.try_tick()`).

### `ServeFuture`

```rust
pub type ServeFuture = Pin<Box<dyn Future<Output = RpcResult<Vec<u8>>> + 'static>>;
```

Defined in `afterglow-rpc` (available on both targets). The generated
`serve_async` returns this type; the impl boxes its async body.

## TS client generation

The `#[rpc]` macro generates a typed `.ts` client when `worker = Type` is set.
For async traits, the TS methods are already `async` (they return `Promises`).
`RpcResult<T>` maps to `T` in TS (errors are thrown). See
[`rpc-macro.md`](rpc-macro.md) for the full TS generation rules.

## Cross-links

- [`assets.md`](assets.md) — path/MIME/confinement (the `AssetRoot` + `resolve`
  boundary the sources build on).
- [`ring-buffer.md`](ring-buffer.md) — sync native transport.
- [`rpc-macro.md`](rpc-macro.md) — `#[rpc]` attribute (sync + async paths).
- [`web-shared-memory.md`](web-shared-memory.md) — the web HTTP dev server.
