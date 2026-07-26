//! deno_core op bridge: exposes spawned native `afterglow-rpc` workers to JS.
//!
//! The shell spawns native worker threads at startup (real OS threads, the
//! `afterglow-rpc::native` ring transport) and registers them here by id. JS
//! calls [`op_afterglow_rpc_call`] — the same `Transport::call(method, args)`
//! surface the web transport exposes — and gets the postcard response payload
//! back as a zero-copy `Uint8Array` (V8 takes ownership of the `Box<[u8]>`
//! backing store, no memcpy). Wakeups are payload-free `unpark`s, exactly as on
//! web (`postMessage`).
//!
//! This is the native realization of the `RpcTransport.call` interface; the
//! generated TS clients are identical across targets, only the transport swaps.
//!
//! Async workers (`AsyncWorkerTransport`) use the same bridge. Source-backed
//! native consumers keep intermediate bytes out of V8 entirely.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::task::{Context, Poll, Wake};
use std::time::Duration;

use afterglow_rpc::Transport;
use afterglow_rpc::native::AsyncWorkerTransport;
use deno_core::convert::Uint8Array;
use deno_core::{JsBuffer, OpState, op2};
use deno_error::JsErrorBox;

/// Registry of spawned native sync workers keyed by a stable id. Stored in
/// `OpState`; sync ops access it on the JS thread (the runtime is single-threaded
/// per isolate). `WorkerTransport` is `Send + !Sync`; that is fine here because
/// only the JS thread ever calls `Transport::call` on it.
pub struct WorkerRegistry {
    workers: HashMap<u32, Box<dyn Transport>>,
    async_workers: HashMap<u32, Arc<AsyncWorkerTransport>>,
    services: HashMap<String, Vec<u32>>,
    pending_async: Arc<AtomicU32>,
}

impl WorkerRegistry {
    pub fn new() -> Self {
        Self {
            workers: HashMap::new(),
            async_workers: HashMap::new(),
            services: HashMap::new(),
            pending_async: Arc::new(AtomicU32::new(0)),
        }
    }
    /// Register a spawned sync worker under `id`.
    pub fn register(&mut self, id: u32, worker: Box<dyn Transport>) -> Option<Box<dyn Transport>> {
        self.workers.insert(id, worker)
    }
    /// Register a spawned async worker under `id`.
    pub fn register_async(&mut self, id: u32, worker: Arc<AsyncWorkerTransport>) {
        self.async_workers.insert(id, worker);
    }
    /// Register an async worker and publish its id under a stable service name.
    /// Names are bootstrap metadata; payload calls still use only the worker's
    /// generated ring transport.
    pub fn register_named_async(
        &mut self,
        service: impl Into<String>,
        id: u32,
        worker: Arc<AsyncWorkerTransport>,
    ) {
        self.register_async(id, worker);
        self.services.entry(service.into()).or_default().push(id);
    }
    /// Return the bootstrap-ordered ids for one service type.
    pub fn service_ids(&self, service: &str) -> &[u32] {
        self.services.get(service).map(Vec::as_slice).unwrap_or(&[])
    }
    /// Number of registered workers.
    pub fn len(&self) -> usize {
        self.workers.len() + self.async_workers.len()
    }
    pub fn is_empty(&self) -> bool {
        self.workers.is_empty() && self.async_workers.is_empty()
    }
    /// Call a registered worker: writes `[method][args]`, blocks for the
    /// response, returns the postcard payload. Dispatches to the sync worker's
    /// `Transport::call` or the async worker's `call_async` + a blocking poll
    /// loop. Same surface as the web transport's `call`.
    pub fn call(&self, worker_id: u32, method: u32, args: &[u8]) -> Result<Vec<u8>, String> {
        if let Some(worker) = self.workers.get(&worker_id) {
            return worker.call(method, args).map_err(|e| e.to_string());
        }
        if let Some(transport) = self.async_workers.get(&worker_id) {
            return block_on_async_call(transport, method, args).map_err(|e| e.to_string());
        }
        Err(format!("unknown worker id {worker_id}"))
    }

    fn call_async(
        &self,
        worker_id: u32,
        method: u32,
        args: &[u8],
    ) -> Result<(afterglow_rpc::native::Oneshot, RpcPending), String> {
        let transport = self
            .async_workers
            .get(&worker_id)
            .ok_or_else(|| format!("worker {worker_id} is not an async worker"))?;
        let future = transport
            .call_async(method, args)
            .map_err(|error| error.to_string())?;
        self.pending_async.fetch_add(1, Ordering::AcqRel);
        Ok((future, RpcPending(self.pending_async.clone())))
    }

    pub fn poll_async(&self) {
        if self.pending_async.load(Ordering::Acquire) == 0 {
            return;
        }
        for transport in self.async_workers.values() {
            transport.poll();
        }
    }

    pub fn has_pending_async(&self) -> bool {
        self.pending_async.load(Ordering::Acquire) != 0
    }
}

struct RpcPending(Arc<AtomicU32>);

impl Drop for RpcPending {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Default for WorkerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// A `Wake` implementation that unparks the calling thread — used by
/// [`block_on_async_call`] to block on an `AsyncWorkerTransport` `Oneshot`.
struct ThreadWaker(std::thread::Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

/// Block the current thread on an async worker call: writes the request via
/// `call_async`, then drives `AsyncWorkerTransport::poll` (which drains the
/// response ring + resolves the `Oneshot`) until the future is ready. The
/// `park_timeout` bounds missed-unpark latency (the worker unparks the client
/// thread after writing a completion). This mirrors the web transport's
/// blocking `call` (one in-flight call, JS thread parked until the response).
fn block_on_async_call(
    transport: &AsyncWorkerTransport,
    method: u32,
    args: &[u8],
) -> afterglow_rpc::RpcResult<Vec<u8>> {
    use std::pin::Pin;
    let mut oneshot = transport.call_async(method, args)?;
    let waker = std::task::Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    loop {
        transport.poll();
        match Pin::new(&mut oneshot).poll(&mut cx) {
            Poll::Ready(res) => return res,
            Poll::Pending => std::thread::park_timeout(Duration::from_micros(500)),
        }
    }
}

/// Typed native asset client. JS-visible payloads are bounded ring responses;
/// source-backed texture workers keep VT source bytes entirely native.
pub struct NativeAssetService {
    client: afterglow_assets_worker::AssetLoaderClient,
    pending: Arc<AtomicU32>,
}

struct NativeAssetPending(Arc<AtomicU32>);

impl Drop for NativeAssetPending {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

pub fn register_native_assets(state: &mut OpState, root: afterglow_assets::AssetRoot) {
    afterglow_assets_worker::AssetLoaderWorker::set_asset_root(root);
    let client = afterglow_assets_worker::AssetLoaderClient::spawn_worker()
        .expect("spawn native asset worker");
    state.put(NativeAssetService {
        client,
        pending: Arc::new(AtomicU32::new(0)),
    });
}

pub fn poll_native_assets(state: &OpState) {
    if let Some(assets) = state.try_borrow::<NativeAssetService>() {
        assets.client.poll();
    }
}

pub fn native_assets_pending(state: &OpState) -> bool {
    state
        .try_borrow::<NativeAssetService>()
        .is_some_and(|assets| assets.pending.load(Ordering::Acquire) != 0)
}

#[op2]
pub async fn op_native_asset_size(
    state: Rc<RefCell<OpState>>,
    #[string] path: String,
) -> Result<f64, JsErrorBox> {
    let (client, pending_counter) = {
        let state = state.borrow();
        let assets = state.borrow::<NativeAssetService>();
        (assets.client.clone(), assets.pending.clone())
    };
    let future = client
        .size(path)
        .map_err(|error| JsErrorBox::generic(error.to_string()))?;
    pending_counter.fetch_add(1, Ordering::AcqRel);
    let _pending = NativeAssetPending(pending_counter);
    let size = future
        .await
        .map_err(|error| JsErrorBox::generic(error.to_string()))?;
    if size > 9_007_199_254_740_991 {
        return Err(JsErrorBox::generic(
            "asset size exceeds JavaScript safe integer",
        ));
    }
    Ok(size as f64)
}

#[op2]
pub async fn op_native_asset_read_copy(
    state: Rc<RefCell<OpState>>,
    #[string] path: String,
    #[bigint] offset: u64,
    len: u32,
) -> Result<Uint8Array, JsErrorBox> {
    let (client, pending_counter) = {
        let state = state.borrow();
        let assets = state.borrow::<NativeAssetService>();
        (assets.client.clone(), assets.pending.clone())
    };
    let future = client
        .read(path, offset, len)
        .map_err(|error| JsErrorBox::generic(error.to_string()))?;
    pending_counter.fetch_add(1, Ordering::AcqRel);
    let _pending = NativeAssetPending(pending_counter);
    let bytes = future
        .await
        .map_err(|error| JsErrorBox::generic(error.to_string()))?;
    Ok(bytes.into())
}

/// Synchronous native worker call: writes `[method:u32][args]`, blocks for the
/// response (the worker is unparked, processes, writes the response ring, then
/// unparks the JS thread), and returns the postcard payload. Same semantics as
/// the web transport's blocking `call` (one in-flight SPSC call per worker).
///
/// The response `Vec<u8>` becomes a V8 `Uint8Array` via ownership transfer
/// (`new_backing_store_from_boxed_slice`) — no memcpy into V8.
#[op2]
pub fn op_afterglow_rpc_call(
    state: &mut OpState,
    worker_id: u32,
    method: u32,
    #[buffer] args: &[u8],
) -> Result<Uint8Array, JsErrorBox> {
    let registry = state.borrow::<WorkerRegistry>();
    let resp = registry
        .call(worker_id, method, args)
        .map_err(JsErrorBox::generic)?;
    Ok(resp.into())
}

/// Non-blocking native async-worker call. The ring request is written
/// immediately; host frame polling resolves the returned promise.
/// Resolve configured worker ids by service name. This removes platform worker
/// numbering from authored TypeScript; the application bootstrap remains the
/// sole owner of concrete ids and process topology.
#[op2]
#[serde]
pub fn op_afterglow_worker_ids(state: &mut OpState, #[string] service: String) -> Vec<u32> {
    state
        .borrow::<WorkerRegistry>()
        .service_ids(&service)
        .to_vec()
}

#[op2]
pub async fn op_afterglow_rpc_call_async(
    state: Rc<RefCell<OpState>>,
    worker_id: u32,
    method: u32,
    #[buffer] args: JsBuffer,
) -> Result<Uint8Array, JsErrorBox> {
    let (future, pending) = {
        let state = state.borrow();
        state
            .borrow::<WorkerRegistry>()
            .call_async(worker_id, method, &args)
            .map_err(JsErrorBox::generic)?
    };
    let _pending = pending;
    let response = future
        .await
        .map_err(|error| JsErrorBox::generic(error.to_string()))?;
    Ok(response.into())
}

pub fn poll_async_workers(state: &OpState) {
    state.borrow::<WorkerRegistry>().poll_async();
}

pub fn async_workers_pending(state: &OpState) -> bool {
    state.borrow::<WorkerRegistry>().has_pending_async()
}

/// Compile-time proof that `wgpu_core::Global` is `Send + Sync`, so an
/// `Arc<Global>` can be shared with worker threads and a worker can derive a
/// `wgpu::Device`/`Queue` via `wgpu::Device::from_shared_core` (the same path
/// the shell's `GpuHudPresenter` uses) to upload directly to the GPU.
const _: () = {
    fn _assert_send_sync<T: Send + Sync>() {}
    fn _global() {
        _assert_send_sync::<wgpu_core::global::Global>();
    }
};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use afterglow_rpc::encode;
    use afterglow_rpc_demo::{PhysicsServer, PhysicsWorker};

    fn register_physics(registry: &mut WorkerRegistry, id: u32) {
        let (transport, _events) = afterglow_rpc::native::spawn_worker_loop(
            PhysicsWorker,
            1 << 20,
            |worker: &mut PhysicsWorker, method, args| worker.serve(method, args),
        )
        .unwrap();
        registry.register(id, Box::new(transport));
    }

    fn register_texture(registry: &mut WorkerRegistry, id: u32) {
        use afterglow_texture::{TextureServer, TextureWorker};
        let (transport, _events) = afterglow_rpc::native::spawn_async_worker_loop(
            TextureWorker::default(),
            1 << 20,
            |worker: &TextureWorker, method, args| worker.serve_async(method, args),
        )
        .unwrap();
        registry.register_named_async("texture", id, Arc::new(transport));
    }

    /// Direct round-trip: spawn Physics natively, encode a `step` call as
    /// postcard, call through the registry, decode the response. Proves the op
    /// bridge's core + native worker composition without a JsRuntime.
    #[test]
    fn op_call_round_trips_through_native_physics_worker() {
        let mut registry = WorkerRegistry::new();
        register_physics(&mut registry, 0);

        // Physics::step(vec![0.0, 1.0, 2.0], 0.5) -> vec![0.5, 1.5, 2.5]
        // Method id 0 (step is the first method). Args are postcard-encoded
        // (Vec<f32>, f32) — the same wire format the generated TS client emits.
        let args = encode(&(vec![0.0_f32, 1.0, 2.0], 0.5_f32)).unwrap();
        // `WorkerRegistry::call` (via `Transport::call`) already unwraps the
        // `Response` envelope, so the result is the bare postcard payload.
        let payload = registry.call(0, 0, &args).unwrap();
        let result: Vec<f32> = afterglow_rpc::decode(&payload).unwrap();
        assert_eq!(result, vec![0.5, 1.5, 2.5]);
    }

    /// An unknown worker id is a clean error, not a panic.
    #[test]
    fn unknown_worker_id_is_an_error() {
        let registry = WorkerRegistry::new();
        let args = encode(&0_u32).unwrap();
        let err = registry.call(999, 0, &args).unwrap_err();
        assert!(err.contains("unknown worker id 999"));
    }

    /// A server error from the worker is surfaced through the `Response`
    /// envelope, not swallowed.
    #[test]
    fn unknown_method_surfaces_a_server_error() {
        let mut registry = WorkerRegistry::new();
        register_physics(&mut registry, 0);
        // Method 99 is not part of the Physics trait.
        let err = registry.call(0, 99, &[]).unwrap_err();
        assert!(err.contains("unknown method"));
    }

    /// Sanity: the typed `PhysicsClient` path the shell mirrors is itself sound.
    #[test]
    fn typed_client_spawn_round_trip() {
        let (client, _events) =
            afterglow_rpc_demo::PhysicsClient::spawn_worker(PhysicsWorker).unwrap();
        let next = client.step(vec![0.0, 1.0, 2.0], 0.5).unwrap();
        assert_eq!(next, vec![0.5, 1.5, 2.5]);
    }

    /// An async worker (`async fn` methods) round-trips through the same
    /// `WorkerRegistry::call` via the blocking-poll wrapper — proving the
    /// op bridge handles async workers (assets/texture/audio) as well as sync.
    #[test]
    fn async_worker_round_trips_through_blocking_poll() {
        // A trivial async serve fn: echoes the args back as the payload.
        fn echo_async(_s: &(), _method: u32, args: &[u8]) -> afterglow_rpc::ServeFuture {
            let args = args.to_vec();
            Box::pin(async move { Ok(args) })
        }
        let (transport, _events) =
            afterglow_rpc::native::spawn_async_worker_loop((), 1 << 20, echo_async).unwrap();
        let mut registry = WorkerRegistry::new();
        registry.register_async(0, Arc::new(transport));
        // The serve fn returns the args verbatim as the Ok payload.
        let args = vec![1_u8, 2, 3];
        let resp = registry.call(0, 0, &args).unwrap();
        assert_eq!(resp, vec![1, 2, 3]);
    }

    #[test]
    fn named_services_publish_bootstrap_order_without_js_worker_constants() {
        let mut registry = WorkerRegistry::new();
        register_texture(&mut registry, 17);
        register_texture(&mut registry, 4);
        assert_eq!(registry.service_ids("texture"), &[17, 4]);
        assert!(registry.service_ids("missing").is_empty());
    }

    /// A real engine service (the `Texture` transcoder — async `#[rpc]`) composes
    /// through the op bridge + dispatches: calling an unknown method surfaces
    /// "unknown method" from the texture worker, proving a real (non-demo)
    /// service is spawned natively + reached via `WorkerRegistry::call`.
    #[test]
    fn real_texture_worker_composes_and_dispatches() {
        use afterglow_texture::{TextureServer, TextureWorker};
        let (transport, _events) = afterglow_rpc::native::spawn_async_worker_loop(
            TextureWorker::default(),
            1 << 20,
            |s: &TextureWorker, m, a| s.serve_async(m, a),
        )
        .unwrap();
        let mut registry = WorkerRegistry::new();
        registry.register_async(0, Arc::new(transport));
        // Method 99 is not part of the Texture trait → the worker surfaces
        // "unknown method" through the Response envelope.
        let err = registry.call(0, 99, &[]).unwrap_err();
        assert!(err.contains("unknown method"), "got: {err}");
    }
}
