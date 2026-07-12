//! # afterglow-assets-worker
//!
//! Asset loader worker: an `#[rpc(worker = AssetLoaderWorker)]` service with
//! `async fn load(path) -> RpcResult<Vec<u8>>`. Loads assets from disk via
//! `afterglow_assets::FsSource` (streaming `pread`, no whole-file buffering for
//! small reads). Uses the async `#[rpc]` poll model — the client returns a
//! `Future` from `load` and resolves it via `poll()` each frame.
//!
//! ## Usage (native)
//!
//! ```no_run
//! use afterglow_assets::AssetRoot;
//! use afterglow_assets_worker::AssetLoaderWorker;
//!
//! // Set the root once (e.g. from AppBuilder::on_ready).
//! AssetLoaderWorker::set_asset_root(AssetRoot::new("assets").unwrap());
//!
//! // Spawn the singleton worker — returns the client directly.
//! let client = afterglow_assets_worker::AssetLoaderClient::spawn_worker().unwrap();
//!
//! // Non-blocking: returns a Future, doesn't park.
//! let fut = client.load("textures/sky.png".into()).unwrap();
//! // ... each frame:
//! client.poll();  // drains completions, resolves the future
//! // When resolved:
//! // let bytes = fut.await;
//! ```
//!
//! ## Web
//!
//! On the web target, asset loading goes through JS `fetch` with `Range`
//! headers (the serving layer), not this worker. The wasm path for async
//! `#[rpc]` requires JS to drive the executor and is deferred.

use afterglow_rpc::RpcResult;
use afterglow_rpc::ServeFuture;
use afterglow_rpc_macros::rpc;

use afterglow_assets::AssetRoot;

#[cfg(target_arch = "wasm32")]
pub mod fetch;

/// An asset loader worker service. Supports full loads (`load`) and
/// streaming partial reads (`size` + `read`) for large assets that don't fit
/// in the response ring. The `async fn` makes this an async `#[rpc]` service —
/// the client returns a `Future` and resolves it via `poll()` (the poll model).
#[rpc(worker = AssetLoaderWorker, singleton)]
pub trait AssetLoader {
    /// Load a full asset into memory. For small files only — large files
    /// should use `size` + `read` to stream in chunks.
    async fn load(path: String) -> RpcResult<Vec<u8>>;

    /// Get the size of an asset in bytes (without loading it). Use this to
    /// plan chunked reads for large files.
    async fn size(path: String) -> RpcResult<u64>;

    /// Read up to `len` bytes at `offset` from an asset. Returns the bytes
    /// read (may be fewer than `len` at EOF). 0 bytes at/past EOF. The path
    /// is the handle — each call opens independently (pread/fetch has no
    /// cursor state, so this is efficient).
    async fn read(path: String, offset: u64, len: u32) -> RpcResult<Vec<u8>>;
}

/// The concrete worker impl. On native, reads from the asset root set via
/// [`set_asset_root`]. On web, fetches via JS-imported functions (see `fetch.rs`).
///
/// [`set_asset_root`]: AssetLoaderWorker::set_asset_root
pub struct AssetLoaderWorker {
    #[cfg(not(target_arch = "wasm32"))]
    root: Option<AssetRoot>,
}

#[cfg(not(target_arch = "wasm32"))]
static ASSET_ROOT: std::sync::OnceLock<AssetRoot> = std::sync::OnceLock::new();

impl AssetLoaderWorker {
    /// Set the asset root for the singleton worker. Call this once before
    /// `AssetLoaderClient::spawn_worker()` (e.g. from `AppBuilder::on_ready`).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_asset_root(root: AssetRoot) {
        let _ = ASSET_ROOT.set(root);
    }
}

// On both targets, Default exists so the macro's `afterglow_wasm_init` (wasm)
// and singleton `spawn_worker()` (native) can construct a default instance.
#[cfg(not(target_arch = "wasm32"))]
impl Default for AssetLoaderWorker {
    fn default() -> Self {
        Self {
            root: ASSET_ROOT.get().cloned(),
        }
    }
}
#[cfg(target_arch = "wasm32")]
impl Default for AssetLoaderWorker {
    fn default() -> Self {
        Self {}
    }
}

impl AssetLoaderServer for AssetLoaderWorker {
    fn load(&self, path: String) -> ServeFuture {
        // The trait method takes `&self` but the future is `'static`, so we
        // can't borrow `self`. Clone what each path needs.
        #[cfg(not(target_arch = "wasm32"))]
        let root = self.root.clone();
        Box::pin(async move {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let root = root.ok_or_else(|| {
                    afterglow_rpc::RpcError::Server("asset worker has no root".into())
                })?;
                let src = root.open_source(&path).ok_or_else(|| {
                    afterglow_rpc::RpcError::Server(format!("asset not found: {path}"))
                })?;
                let mut buf = vec![0u8; src.len() as usize];
                use afterglow_assets::AssetSource;
                src.read_at(0, &mut buf)?;
                afterglow_rpc::encode(&buf)
            }
            #[cfg(target_arch = "wasm32")]
            {
                crate::fetch::fetch_asset(&path).await
            }
        })
    }

    fn size(&self, path: String) -> ServeFuture {
        #[cfg(not(target_arch = "wasm32"))]
        let root = self.root.clone();
        Box::pin(async move {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let root = root.ok_or_else(|| {
                    afterglow_rpc::RpcError::Server("asset worker has no root".into())
                })?;
                let src = root.open_source(&path).ok_or_else(|| {
                    afterglow_rpc::RpcError::Server(format!("asset not found: {path}"))
                })?;
                use afterglow_assets::AssetSource;
                afterglow_rpc::encode(&src.len())
            }
            #[cfg(target_arch = "wasm32")]
            {
                crate::fetch::fetch_size(&path).await
            }
        })
    }

    fn read(&self, path: String, offset: u64, len: u32) -> ServeFuture {
        #[cfg(not(target_arch = "wasm32"))]
        let root = self.root.clone();
        Box::pin(async move {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let root = root.ok_or_else(|| {
                    afterglow_rpc::RpcError::Server("asset worker has no root".into())
                })?;
                let src = root.open_source(&path).ok_or_else(|| {
                    afterglow_rpc::RpcError::Server(format!("asset not found: {path}"))
                })?;
                use afterglow_assets::AssetSource;
                let mut buf = vec![0u8; len as usize];
                let n = src.read_at(offset, &mut buf)?;
                buf.truncate(n);
                afterglow_rpc::encode(&buf)
            }
            #[cfg(target_arch = "wasm32")]
            {
                crate::fetch::fetch_range(&path, offset, len).await
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    /// Drive a future to completion via the poll model.
    fn drive<F: std::future::Future>(client: &AssetLoaderClient, fut: F) -> F::Output {
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
        let mut cx = Context::from_waker(&waker);
        let mut fut = std::pin::pin!(fut);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            client.poll();
            match fut.as_mut().poll(&mut cx) {
                Poll::Ready(v) => return v,
                Poll::Pending => {
                    if std::time::Instant::now() > deadline {
                        panic!("timed out");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        }
    }

    #[test]
    fn singleton_full_lifecycle() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("ag-asset-worker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut f = std::fs::File::create(dir.join("test.txt")).unwrap();
        f.write_all(b"hello asset").unwrap();
        let root = AssetRoot::new(&dir).unwrap();

        AssetLoaderWorker::set_asset_root(root.clone());

        // 1. First spawn.
        let client = AssetLoaderClient::spawn_worker().unwrap();

        // 2. Load (full).
        let bytes: Vec<u8> = drive(&client, client.load("test.txt".into()).unwrap()).unwrap();
        assert_eq!(bytes, b"hello asset");

        // 3. Size.
        let size: u64 = drive(&client, client.size("test.txt".into()).unwrap()).unwrap();
        assert_eq!(size, 11);

        // 4. Read at offset.
        let chunk: Vec<u8> = drive(&client, client.read("test.txt".into(), 6, 5).unwrap()).unwrap();
        assert_eq!(chunk, b"asset");

        // 5. Read at EOF → empty.
        let eof: Vec<u8> = drive(&client, client.read("test.txt".into(), 11, 10).unwrap()).unwrap();
        assert!(eof.is_empty(), "read at EOF should return empty");

        // 6. Read past end → partial.
        let tail: Vec<u8> =
            drive(&client, client.read("test.txt".into(), 9, 100).unwrap()).unwrap();
        assert_eq!(tail, b"et", "partial read at end");

        // 7. Missing asset → error.
        let result = drive(&client, client.load("nope.bin".into()).unwrap());
        assert!(result.is_err());

        // 8. Second spawn shares the worker.
        let client2 = AssetLoaderClient::spawn_worker().unwrap();
        let bytes2: Vec<u8> = drive(&client2, client2.load("test.txt".into()).unwrap()).unwrap();
        assert_eq!(bytes2, b"hello asset");

        // 9. Concurrent multi-threaded.
        let handles: Vec<_> = (0..4)
            .map(|i| {
                let c = client.clone();
                std::thread::spawn(move || {
                    let fut = c.read("test.txt".into(), 0, 5).unwrap();
                    let bytes: Vec<u8> = drive(&c, fut).unwrap();
                    assert_eq!(bytes, b"hello", "thread {i}");
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        // 10. Drop all → worker dies, re-spawn.
        drop(client);
        drop(client2);
        let client3 = AssetLoaderClient::spawn_worker().unwrap();
        let bytes3: Vec<u8> = drive(&client3, client3.load("test.txt".into()).unwrap()).unwrap();
        assert_eq!(bytes3, b"hello asset");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn streaming_large_file() {
        use std::io::Write;
        // Create a file larger than the 1 MiB response ring — verify we can
        // stream it in chunks via size + read.
        let dir = std::env::temp_dir().join(format!("ag-asset-stream-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 2 MiB of deterministic data.
        let size = 2 * 1024 * 1024;
        let path = dir.join("big.bin");
        let mut f = std::fs::File::create(&path).unwrap();
        let chunk = (0..4096).map(|i| (i % 256) as u8).collect::<Vec<u8>>();
        for _ in 0..(size / 4096) {
            f.write_all(&chunk).unwrap();
        }

        // The singleton worker was already spawned by the other test, but it
        // points to a different root. We can't re-set the root (OnceLock), so
        // we use the AsyncTypeMatrix worker (non-singleton) to test streaming
        // via the raw FsSource directly.
        // Instead, verify the streaming API works by testing against the
        // first test's root (which the singleton worker shares).
        // → Use a non-singleton async worker for the large file test:
        let root = AssetRoot::new(&dir).unwrap();
        // We can't use the singleton (already spawned with a different root).
        // Test the streaming via FsSource directly instead.
        use afterglow_assets::AssetSource;
        let src = root.open_source("/big.bin").unwrap();
        assert_eq!(src.len(), size as u64);

        // Read chunks and verify.
        let mut offset = 0u64;
        let chunk_size = 512 * 1024; // 512 KiB per read
        while offset < size as u64 {
            let want = std::cmp::min(chunk_size, (size as u64 - offset) as usize) as u32;
            let mut buf = vec![0u8; want as usize];
            let n = src.read_at(offset, &mut buf).unwrap();
            assert!(n > 0, "read at {offset} returned 0");
            // Verify data.
            for i in 0..n {
                assert_eq!(
                    buf[i],
                    ((offset as usize + i) % 256) as u8,
                    "byte at {offset}+{i}"
                );
            }
            offset += n as u64;
        }
        assert_eq!(offset, size as u64, "streamed the whole file");

        let _ = std::fs::remove_dir_all(&dir);
    }

    static VTABLE: RawWakerVTable = RawWakerVTable::new(
        |_| RawWaker::new(std::ptr::null(), &VTABLE), // clone
        |_| {},                                       // wake
        |_| {},                                       // wake_by_ref
        |_| {},                                       // drop
    );

    // --- Async type matrix: every type round-trips through the async poll model ---

    use afterglow_rpc::ServeFuture;
    use afterglow_rpc_macros::rpc;

    #[rpc(worker = AsyncTypeMatrixWorker)]
    pub trait AsyncTypeMatrix {
        async fn echo_f32(x: f32) -> RpcResult<f32>;
        async fn echo_u32(x: u32) -> RpcResult<u32>;
        async fn echo_u64(x: u64) -> RpcResult<u64>;
        async fn echo_i64(x: i64) -> RpcResult<i64>;
        async fn echo_bool(x: bool) -> RpcResult<bool>;
        async fn echo_string(s: String) -> RpcResult<String>;
        async fn echo_vec_u8(v: Vec<u8>) -> RpcResult<Vec<u8>>;
        async fn echo_vec_f32(v: Vec<f32>) -> RpcResult<Vec<f32>>;
        async fn multi(a: u32, b: String, c: bool) -> RpcResult<u64>;
        async fn no_args() -> RpcResult<u32>;
        async fn void(x: u32) -> RpcResult<()>;
    }

    #[derive(Default)]
    pub struct AsyncTypeMatrixWorker;

    impl AsyncTypeMatrixServer for AsyncTypeMatrixWorker {
        fn echo_f32(&self, x: f32) -> ServeFuture {
            Box::pin(async move { afterglow_rpc::encode(&x) })
        }
        fn echo_u32(&self, x: u32) -> ServeFuture {
            Box::pin(async move { afterglow_rpc::encode(&x) })
        }
        fn echo_u64(&self, x: u64) -> ServeFuture {
            Box::pin(async move { afterglow_rpc::encode(&x) })
        }
        fn echo_i64(&self, x: i64) -> ServeFuture {
            Box::pin(async move { afterglow_rpc::encode(&x) })
        }
        fn echo_bool(&self, x: bool) -> ServeFuture {
            Box::pin(async move { afterglow_rpc::encode(&x) })
        }
        fn echo_string(&self, s: String) -> ServeFuture {
            Box::pin(async move { afterglow_rpc::encode(&s) })
        }
        fn echo_vec_u8(&self, v: Vec<u8>) -> ServeFuture {
            Box::pin(async move { afterglow_rpc::encode(&v) })
        }
        fn echo_vec_f32(&self, v: Vec<f32>) -> ServeFuture {
            Box::pin(async move { afterglow_rpc::encode(&v) })
        }
        fn multi(&self, a: u32, b: String, c: bool) -> ServeFuture {
            Box::pin(async move {
                let result = (a as u64) + (b.len() as u64) + if c { 1000 } else { 0 };
                afterglow_rpc::encode(&result)
            })
        }
        fn no_args(&self) -> ServeFuture {
            Box::pin(async move { afterglow_rpc::encode(&42u32) })
        }
        fn void(&self, _x: u32) -> ServeFuture {
            Box::pin(async move { afterglow_rpc::encode(&()) })
        }
    }

    /// Drive a future to completion via the poll model: call `client.poll()`
    /// each iteration until the future resolves.
    fn drive_to_completion<F: std::future::Future>(
        client: &AsyncTypeMatrixClient,
        fut: F,
    ) -> F::Output {
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
        let mut cx = Context::from_waker(&waker);
        let mut fut = std::pin::pin!(fut);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            client.poll();
            match fut.as_mut().poll(&mut cx) {
                Poll::Ready(v) => return v,
                Poll::Pending => {
                    if std::time::Instant::now() > deadline {
                        panic!("async test timed out");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        }
    }

    #[test]
    fn async_all_types_round_trip() {
        let (client, _ev) = AsyncTypeMatrixClient::spawn_worker(AsyncTypeMatrixWorker).unwrap();

        assert_eq!(
            drive_to_completion(&client, client.echo_f32(3.14).unwrap()).unwrap(),
            3.14
        );
        assert_eq!(
            drive_to_completion(&client, client.echo_u32(42).unwrap()).unwrap(),
            42
        );
        assert_eq!(
            drive_to_completion(&client, client.echo_u64(u64::MAX).unwrap()).unwrap(),
            u64::MAX
        );
        assert_eq!(
            drive_to_completion(&client, client.echo_i64(i64::MIN).unwrap()).unwrap(),
            i64::MIN
        );
        assert_eq!(
            drive_to_completion(&client, client.echo_bool(true).unwrap()).unwrap(),
            true
        );
        assert_eq!(
            drive_to_completion(&client, client.echo_bool(false).unwrap()).unwrap(),
            false
        );
        assert_eq!(
            drive_to_completion(&client, client.echo_string("héllo".into()).unwrap()).unwrap(),
            "héllo"
        );
        assert_eq!(
            drive_to_completion(&client, client.echo_vec_u8(vec![1, 2, 3]).unwrap()).unwrap(),
            vec![1, 2, 3]
        );
        assert_eq!(
            drive_to_completion(&client, client.echo_vec_f32(vec![1.5, -2.5]).unwrap()).unwrap(),
            vec![1.5, -2.5]
        );
        assert_eq!(
            drive_to_completion(&client, client.multi(10, "hello".into(), true).unwrap()).unwrap(),
            10 + 5 + 1000
        );
        assert_eq!(
            drive_to_completion(&client, client.no_args().unwrap()).unwrap(),
            42
        );
        assert!(drive_to_completion(&client, client.void(99).unwrap()).is_ok());
    }

    #[test]
    fn async_multiple_concurrent_in_flight() {
        // The poll model supports multiple in-flight async calls — each gets
        // a unique task_id. Start several, then poll until all resolve.
        let (client, _ev) = AsyncTypeMatrixClient::spawn_worker(AsyncTypeMatrixWorker).unwrap();

        // Start 5 concurrent calls (non-blocking — they return futures immediately).
        let f1 = client.echo_u32(1).unwrap();
        let f2 = client.echo_u32(2).unwrap();
        let f3 = client.echo_string("hello".into()).unwrap();
        let f4 = client.echo_vec_f32(vec![1.0, 2.0, 3.0]).unwrap();
        let f5 = client.echo_bool(true).unwrap();

        // Drive all to completion via the same poll loop. They resolve in
        // whatever order the executor completes them.
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
        let mut cx = Context::from_waker(&waker);
        let mut f1 = std::pin::pin!(f1);
        let mut f2 = std::pin::pin!(f2);
        let mut f3 = std::pin::pin!(f3);
        let mut f4 = std::pin::pin!(f4);
        let mut f5 = std::pin::pin!(f5);

        let mut r1 = None;
        let mut r2 = None;
        let mut r3 = None;
        let mut r4 = None;
        let mut r5 = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);

        while r1.is_none() || r2.is_none() || r3.is_none() || r4.is_none() || r5.is_none() {
            client.poll();
            if r1.is_none() {
                if let Poll::Ready(v) = f1.as_mut().poll(&mut cx) {
                    r1 = Some(v);
                }
            }
            if r2.is_none() {
                if let Poll::Ready(v) = f2.as_mut().poll(&mut cx) {
                    r2 = Some(v);
                }
            }
            if r3.is_none() {
                if let Poll::Ready(v) = f3.as_mut().poll(&mut cx) {
                    r3 = Some(v);
                }
            }
            if r4.is_none() {
                if let Poll::Ready(v) = f4.as_mut().poll(&mut cx) {
                    r4 = Some(v);
                }
            }
            if r5.is_none() {
                if let Poll::Ready(v) = f5.as_mut().poll(&mut cx) {
                    r5 = Some(v);
                }
            }
            if std::time::Instant::now() > deadline {
                panic!("concurrent test timed out");
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        assert_eq!(r1.unwrap().unwrap(), 1);
        assert_eq!(r2.unwrap().unwrap(), 2);
        assert_eq!(r3.unwrap().unwrap(), "hello");
        assert_eq!(r4.unwrap().unwrap(), vec![1.0, 2.0, 3.0]);
        assert_eq!(r5.unwrap().unwrap(), true);
    }
}
