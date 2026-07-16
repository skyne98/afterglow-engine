# `afterglow-assets` + `afterglow-assets-worker` API — streaming assets + async loader

> Status: working; API checked against the 2026-07-12 source.

## Streaming sources (`afterglow-assets::source`)

```rust
pub trait AssetSource: Send + Sync {
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

Reads via `FileExt::read_at` (Unix `pread`, Windows `seek_read`) — no mutex, no
whole-file load. ETag derived from modified-time (weak, mtime-based).

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

Single-range only (`bytes=0-499`, `500-`, `-500`). Multi-range → `Full` (no
multipart). Whitespace-tolerant.

## Serving layer

### CEF scheme (`afterglow-cef/src/resources.rs`)

- `open` → resolves to `Box<dyn AssetSource + Send + Sync>`.
- `response_headers` → sets `Content-Length`, `Accept-Ranges: bytes`, ETag,
  COOP/COEP/CORP.
- `skip(n)` → advances the read offset (CEF's range primitive).
- `read(data_out, n)` → `source.read_at(offset, buf)`, streams from disk.
- No whole-file buffering. Embedded-first (`index.html`), then FS fallback.

### Web HTTP dev server (`afterglow-web/src/dev_server.rs`)

- `handle_request(root, request) -> Response` — parses `Range` header.
- `Range` → `206 Partial Content` + `Content-Range`; no range → `200` full.
- `stream_body(resp, writer, chunk)` — streams via `read_at` in chunks.
- `Accept-Ranges: bytes` on every response (via `CROSS_ORIGIN_HEADERS`).

## JS asset loader — via the worker client (both backends)

The render thread uses the generated `AssetLoaderClient` TS client on **both**
backends. On native, the worker reads from disk via `FsSource`/`pread` and
retains up to 16 open sources in a fixed round-robin cache; repeated container
ranges therefore reuse descriptors without unbounded path growth. On web, the
worker fetches via JS-imported `ag_fetch_start`/`ag_fetch_poll` (the
`fetch.rs` bridge), driven by `async-worker.js`'s tick loop. No separate JS
fetch glue in user code — the worker is the single entry point.

## Engine `AssetStore`

`AssetStore` has a fixed capacity (1,024 by default). `registerAsset(path)`
interns a path during manifest/bootstrap and returns a numeric `AssetId`; it
returns `-1` rather than growing past capacity. `tryLoadAsset(id, parser)` uses
direct `Uint8Array` state and object-handle tables. The game-facing `load(path,
parser)` wrapper performs registration and throws on capacity exhaustion.

States are `Free → Idle → Reading → Parsing → ReadyToPublish → Ready`, with an
`Error` terminal/retry state. Promise callbacks only place `(id, token, kind,
value)` into a preallocated completion ring. `poll()` publishes at most 32
completions by default, so microtask timing cannot mutate visible asset state in
the middle of rendering. Generation tokens reject evicted/stale reads and
dispose stale parsed assets. Queue depth, high-water, and overflow counters are
incremental. `cachedPaths` remains an explicit allocating diagnostic snapshot.

## Asset loader worker (`afterglow-assets-worker`)

```rust
#[rpc(worker = AssetLoaderWorker)]
pub trait AssetLoader {
    async fn load(path: String) -> RpcResult<Vec<u8>>;
}

pub struct AssetLoaderWorker { /* singleton root + 16-source native cache */ }
impl AssetLoaderWorker {
    pub fn set_asset_root(root: AssetRoot);
}
```

Async `#[rpc]` service. `load` reads a file via `FsSource::read_at` and returns
its bytes (postcard-encoded `Vec<u8>`). The client uses the poll model:

```rust
let (client, _events) = AssetLoaderClient::spawn_worker(worker)?;
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
- [`cef-shell.md`](cef-shell.md) — the CEF scheme handler.
- [`web-shared-memory.md`](web-shared-memory.md) — the web HTTP dev server.
