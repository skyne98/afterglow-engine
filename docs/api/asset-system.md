# `afterglow-assets` + `afterglow-assets-worker` API — streaming assets + async loader

> Status: working on public web and the native shell; API checked against the
> 2026-07-26 source. Native Basis page reads and transcoding are composed inside
> real OS workers, so encoded VT source bytes never enter V8. Public-web bulk
> spans still preserve admission order rather than using the separate source-
> sorting helper.

## Engine asset composition

The browser asset surface is deliberately split into three owners:

- `big-container.ts` provides `BigContainer`, an immutable format/index view. It
  owns no workers, renderer state, queues, or platform policy.
- `owned-worker-pool.ts` provides the generic fixed service-lifetime mechanism:
  positive fixed count, reverse startup rollback, reverse idempotent shutdown,
  and first-error reporting.
- `engine-assets.ts` provides `EngineAssets`, the public composition owner. It
  joins a container, the platform texture pool, the page provider, and at most
  one asset/VT store without exposing transports to games.

`EngineAssets.open()` requires `transcodeQueueCapacity`, `maxPendingPages`, `maxPendingBytes`,
`urgentBatchDeadlineMs`, `focusBatchDeadlineMs`,
`peripheralBatchDeadlineMs`, `maxHeaderBytes`, and target GPU format. The
waiting texture capacity plus active worker count must cover every admitted VT
page, so capacity pressure is deferred by admission rather than reported as a
page failure. Optional telemetry connects startup, platform reads, worker round
trips, transcode, mesh optimization, and VT publication. Header validation and
`BigContainer` construction complete before any worker starts.

The public browser barrel is `web/src/engine/assets/index.ts`. The policy-free
`readBigHeader(source, path, maxHeaderBytes)` primitive performs the shared
bounded prefix/header read used by both VT/model archives and static meshes; no
consumer duplicates container validation. Public web creates typed transcoder clients through `spawnThreaded()`.
Games never construct `Rpc`, select worker scripts, use numeric worker IDs, or
terminate raw transports. `createTranscoder` remains a test/platform injection
boundary.

**Native target:** the application bootstrap registers named `texture` and
`meshopt` services in the shell's generic `WorkerRegistry`. TypeScript resolves
the bootstrap manifest instead of embedding worker IDs. Each texture worker
opens the confined BIG source once through `AssetSourceTable`; runtime page jobs
carry only `{source, offset, length, format}`. The worker performs `pread` and
Basis transcode in native memory and returns only the final GPU-format page.
The native application spawns `min(physical cores, 16)` texture workers and
`EngineAssets` consumes at most `maxPendingPages` entries from that manifest.
On the current 16-core/32-thread host this is 16 workers. Public web retains its
bounded two-to-four-worker Fetch/WASM profile. `workerCount` is only an optional
test/profile override; normal games do not choose platform topology.

`createAssetStore(capacity, completionsPerPoll)` starts and owns the standard
mesh optimizer and binds a fixed-capacity `AssetStore` to `BigContainer`'s raw
loader. `EngineAssets` creates at most one `VirtualTextureStore` and one
`AssetStore`. `close()` disposes stores, closes the page provider, then closes
mesh and texture services in reverse order. There is no persistent derived-page
cache or storage fallback.

```ts
const engineAssets = await EngineAssets.open({
  containerPath: 'world.big',
  format,
  transcodeQueueCapacity: 16,
  maxPendingPages: 16,
  maxPendingBytes: 2 * 1024 * 1024,
  urgentBatchDeadlineMs: 1,
  focusBatchDeadlineMs: 16,
  peripheralBatchDeadlineMs: 64, // provisional until the 32/48/64 GPU gate
  maxHeaderBytes: 2 * 1024 * 1024,
  telemetry: runtime.telemetry,
});
const assets = await engineAssets.createAssetStore(64, 8);
const store = engineAssets.createVirtualTextureStore(device, tuning);
```

`createFetchRangeLoader()` also exposes `readBulk(path, ranges)`. Public web
uses standard multipart HTTP ranges for both singleton and bulk reads; the
former CEF native shared-message bridge has been removed. The public-web page
provider preserves scheduler/admission order. It does **not** currently
source-sort spans. `createPageRangeReader()` contains a separate source-sorting
implementation and restores caller order by page index, but the public-web
provider does not call it. The web page provider owns a fixed 256-slot three-tier queue: mip+2 parent misses open a
non-resettable 1 ms window; high-importance exact pages use 16 ms; lower-
importance exact pages currently use a provisional 64 ms peripheral window.
Each response is at most 4 MiB
and at most two responses / 8 MiB are in flight. `close()` rejects queued work
and prevents late raw bytes from entering a closed transcoder pool. Stable
telemetry reports queue depth, in-flight response bytes, urgent/focus/peripheral
batches, rejects, and cancellations.

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

### `AssetSourceTable`

```rust
pub struct AssetSourceHandle(u32); // slot + generation
pub struct AssetSourceTable { /* fixed retained source slots */ }
impl AssetSourceTable {
    pub fn new(capacity: usize) -> Self;
    pub fn open(&mut self, provider: &dyn AssetSourceProvider, path: &str)
        -> Option<AssetSourceHandle>;
    pub fn read_at(&self, handle: AssetSourceHandle, offset: u64, output: &mut [u8])
        -> Option<io::Result<usize>>;
    pub fn clear(&mut self);
}
```

`open` is bootstrap-only and may allocate a retained path/source. Runtime reads
use a numeric handle, perform no path resolution, and cannot grow the table.
`clear` advances every generation, so stale descriptors fail rather than
addressing a replacement source.

### Native JS-visible range adapter

`createPlatformRangeLoader()` selects the native asset ops under
`afterglow-shell`. JS-visible bytes use generated `AssetLoaderClient::read`
responses, split into at most 512 KiB ring payloads. Returned arrays are
V8-owned; there is no reusable native arena lease whose capacity depends on
V8 garbage collection. Native Basis VT pages bypass this adapter entirely and
use the source-backed texture service. Public web routes singleton and bulk
reads through standards-based HTTP Fetch + Range.

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

### Native shell asset loader

The shell confines its asset root once, starts the generated native asset client
on a real OS thread, and exposes bounded `size`/`read` ops. Concrete texture and
mesh worker composition belongs to the application bootstrap; `rpc_bridge.rs`
contains only the generic registry and op adapter. The previous CEF scheme
handler has been removed.

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

## Asset-loader worker versus texture-source composition

JS-visible native headers, resident textures, and raw model ranges use the
generated native `AssetLoaderClient`. Native Basis VT pages take the more direct
source-backed texture path: each texture worker retains a confined numeric
source handle and performs `pread` plus transcode without an intermediate V8
payload. Both compose the generic `afterglow-assets` source primitives; neither
instantiates WASM on the native target. Public web continues to use serving-
layer Fetch for BIG ranges.

## Engine `AssetStore`

`AssetStore` has a fixed capacity (1,024 by default). `registerAsset(path)`
interns a path during manifest/bootstrap and returns a numeric `AssetId`; it
returns `-1` rather than growing past capacity. `tryLoadAsset(id, parser)` uses
direct `Uint8Array` state and object-handle tables. The game-facing `load(path,
parser)` wrapper performs registration and throws on capacity exhaustion.

## Source-backed texture worker

`afterglow-texture::Texture` retains its byte-transform methods and adds two
native source-composition methods:

```rust
async fn open_source(path: String) -> RpcResult<u32>;
async fn transcode_range(
    source: u32, offset: u64, len: u32, target_format: u32,
) -> RpcResult<Vec<u8>>;
```

`open_source` is bootstrap-only and returns an `AssetSourceTable` generational
handle local to that worker. Each worker preallocates one 4 MiB input scratch;
`transcode_range` reads exactly `len` confined bytes into it, rejects overflow,
stale handles, or truncation, and passes the bounded slice to the normal Basis
transcoder. The wasm implementation rejects these methods;
public web uses `transcode(data, format)` after Fetch. The generated TS client
exposes `openSource` and `transcodeRange`, but only the platform adapter calls
them.

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

pub struct AssetLoaderWorker { /* singleton fixed-capacity AssetSourceCache */ }
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
   unparks the client. A full response ring applies bounded retry/backpressure;
   a completion is never dropped.
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
