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
//! Async workers (`AsyncWorkerTransport`) and the shared-arena zero-copy handle
//! path (V8 external ArrayBuffer over an `afterglow_rpc::handle::Arena` slot)
//! are added on top of this sync foundation.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::task::{Context, Poll, Wake};
use std::time::Duration;

use afterglow_rpc::handle::Arena;
use afterglow_rpc::native::AsyncWorkerTransport;
use afterglow_rpc::{Handle, Transport};
use deno_core::convert::Uint8Array;
use deno_core::v8;
use deno_core::{JsBuffer, OpState, op2};
use deno_error::JsErrorBox;

/// Registry of spawned native sync workers keyed by a stable id. Stored in
/// `OpState`; sync ops access it on the JS thread (the runtime is single-threaded
/// per isolate). `WorkerTransport` is `Send + !Sync`; that is fine here because
/// only the JS thread ever calls `Transport::call` on it.
pub struct WorkerRegistry {
    workers: HashMap<u32, Box<dyn Transport>>,
    async_workers: HashMap<u32, Arc<AsyncWorkerTransport>>,
    pending_async: Arc<AtomicU32>,
}

impl WorkerRegistry {
    pub fn new() -> Self {
        Self {
            workers: HashMap::new(),
            async_workers: HashMap::new(),
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

/// Registry of shared arenas keyed by `region` id. Workers hand off `Handle`s
/// into these arenas; [`op_afterglow_arena_view`] exposes a slot to JS as a V8
/// `Uint8Array` backed *externally* by the slot's memory (zero copy — JS reads
/// the worker's bytes in place). When JS GCs the view, the backing store's
/// deleter releases the slot back to the arena.
pub struct ArenaRegistry {
    arenas: HashMap<u32, Arc<Arena>>,
}

impl ArenaRegistry {
    pub fn new() -> Self {
        Self {
            arenas: HashMap::new(),
        }
    }
    pub fn register(&mut self, region: u32, arena: Arc<Arena>) {
        self.arenas.insert(region, arena);
    }
    pub fn get(&self, region: u32) -> Option<Arc<Arena>> {
        self.arenas.get(&region).cloned()
    }
}

impl Default for ArenaRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub const NATIVE_ASSET_ARENA_REGION: u32 = 2;
pub const NATIVE_ASSET_ARENA_SLOTS: usize = 16;
pub const NATIVE_ASSET_ARENA_SLOT_BYTES: usize = 4 * 1024 * 1024;

/// Typed native asset client plus bounded shared-memory arena. The worker writes
/// payloads into arena slots and returns only generational handles over RPC.
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
    let arena = Arena::new(
        NATIVE_ASSET_ARENA_REGION,
        NATIVE_ASSET_ARENA_SLOTS,
        NATIVE_ASSET_ARENA_SLOT_BYTES,
    );
    afterglow_assets_worker::AssetLoaderWorker::set_asset_root(root);
    afterglow_assets_worker::AssetLoaderWorker::set_native_arena(arena.clone());
    let client = afterglow_assets_worker::AssetLoaderClient::spawn_worker()
        .expect("spawn native asset worker");
    state
        .borrow_mut::<ArenaRegistry>()
        .register(NATIVE_ASSET_ARENA_REGION, arena);
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

#[op2]
#[serde]
pub async fn op_native_asset_read_handle(
    state: Rc<RefCell<OpState>>,
    #[string] path: String,
    #[bigint] offset: u64,
    len: u32,
) -> Result<Vec<u32>, JsErrorBox> {
    let (client, pending_counter) = {
        let state = state.borrow();
        let assets = state.borrow::<NativeAssetService>();
        (assets.client.clone(), assets.pending.clone())
    };
    let future = client
        .read_handle(path, offset, len)
        .map_err(|error| JsErrorBox::generic(error.to_string()))?;
    pending_counter.fetch_add(1, Ordering::AcqRel);
    let _pending = NativeAssetPending(pending_counter);
    future
        .await
        .map_err(|error| JsErrorBox::generic(error.to_string()))
}

#[op2]
#[serde]
pub async fn op_native_asset_read_many_handle(
    state: Rc<RefCell<OpState>>,
    #[string] path: String,
    #[buffer] spans: JsBuffer,
) -> Result<Vec<u32>, JsErrorBox> {
    let (client, pending_counter) = {
        let state = state.borrow();
        let assets = state.borrow::<NativeAssetService>();
        (assets.client.clone(), assets.pending.clone())
    };
    let future = client
        .read_many_handle(path, spans.to_vec())
        .map_err(|error| JsErrorBox::generic(error.to_string()))?;
    pending_counter.fetch_add(1, Ordering::AcqRel);
    let _pending = NativeAssetPending(pending_counter);
    future
        .await
        .map_err(|error| JsErrorBox::generic(error.to_string()))
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

/// Spawn the reference `Physics` worker natively and register it under `id`.
/// Used by the shell startup and the bridge tests.
pub fn register_physics(registry: &mut WorkerRegistry, id: u32) {
    use afterglow_rpc_demo::{PhysicsServer, PhysicsWorker};
    let (transport, _events) = afterglow_rpc::native::spawn_worker_loop(
        PhysicsWorker,
        1 << 20,
        |s: &mut PhysicsWorker, m, a| s.serve(m, a),
    )
    .expect("spawn Physics worker");
    registry.register(id, Box::new(transport));
}

/// Spawn the real `Texture` transcoder worker natively (an async `#[rpc]` engine
/// service — Basis → BC7/ASTC/etc.) and register it under `id`. This is a real
/// engine service composed through the op bridge, not a demo: JS calls it via
/// `op_afterglow_rpc_call` + `block_on_async_call` drives the async poll.
pub fn register_texture(registry: &mut WorkerRegistry, id: u32) {
    use afterglow_texture::{TextureServer, TextureWorker};
    let (transport, _events) = afterglow_rpc::native::spawn_async_worker_loop(
        TextureWorker,
        1 << 20,
        |s: &TextureWorker, m, a| s.serve_async(m, a),
    )
    .expect("spawn Texture worker");
    registry.register_async(id, std::sync::Arc::new(transport));
}

/// Spawn the native mesh optimizer worker and register it under `id`.
pub fn register_meshopt(registry: &mut WorkerRegistry, id: u32) {
    use afterglow_meshopt::{MeshoptServer, MeshoptWorker};
    let (transport, _events) = afterglow_rpc::native::spawn_async_worker_loop(
        MeshoptWorker,
        1 << 20,
        |s: &MeshoptWorker, m, a| s.serve_async(m, a),
    )
    .expect("spawn Meshopt worker");
    registry.register_async(id, Arc::new(transport));
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

/// V8 backing-store deleter for an arena slot leased to JS. Runs when V8 GCs
/// the `ArrayBuffer`; releases the slot back to the arena (Reading -> Free +
/// generation advance) so it can be reused.
extern "C" fn arena_slot_deleter(_ptr: *mut c_void, _len: usize, data: *mut c_void) {
    if data.is_null() {
        return;
    }
    // SAFETY: `data` was created by `Box::into_raw` in `op_afterglow_arena_view`;
    // this deleter runs exactly once per backing store.
    let (arena, handle) = *unsafe { Box::from_raw(data as *mut (Arc<Arena>, Handle)) };
    arena.release_read(handle);
}

/// Zero-copy arena view: leases the slot referenced by `handle` and returns a
/// `Uint8Array` backed *externally* by the slot's memory. JS reads the worker's
/// bytes in place — no copy into V8. When JS drops the view, V8's backing-store
/// deleter releases the slot back to the arena.
#[op2]
pub fn op_afterglow_arena_view<'s, 'i>(
    state: &mut OpState,
    scope: &mut v8::PinScope<'s, 'i>,
    #[serde] handle: Handle,
) -> Result<v8::Local<'s, v8::Uint8Array>, JsErrorBox> {
    let arena = state
        .borrow::<ArenaRegistry>()
        .get(handle.region)
        .ok_or_else(|| JsErrorBox::generic(format!("unknown arena region {}", handle.region)))?;
    let (ptr, len) = arena
        .lease_read(handle)
        .ok_or_else(|| JsErrorBox::generic("stale or invalid arena handle"))?;
    // Keep the arena alive until the deleter runs (JS may outlive the op call).
    let deleter_data = Box::into_raw(Box::new((arena.clone(), handle))) as *mut c_void;
    // SAFETY: `ptr` points to `len` valid bytes in the arena slot, exclusively
    // leased to this reader until `arena_slot_deleter` releases it.
    let backing = unsafe {
        v8::ArrayBuffer::new_backing_store_from_ptr(
            ptr as *mut c_void,
            len,
            arena_slot_deleter,
            deleter_data,
        )
    };
    let shared = backing.make_shared();
    let ab = v8::ArrayBuffer::with_backing_store(scope, &shared);
    v8::Uint8Array::new(scope, ab, 0, len)
        .ok_or_else(|| JsErrorBox::generic("failed to create Uint8Array"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use afterglow_rpc::encode;
    use afterglow_rpc_demo::PhysicsWorker;

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

    /// A real engine service (the `Texture` transcoder — async `#[rpc]`) composes
    /// through the op bridge + dispatches: calling an unknown method surfaces
    /// "unknown method" from the texture worker, proving a real (non-demo)
    /// service is spawned natively + reached via `WorkerRegistry::call`.
    #[test]
    fn real_texture_worker_composes_and_dispatches() {
        use afterglow_texture::{TextureServer, TextureWorker};
        let (transport, _events) = afterglow_rpc::native::spawn_async_worker_loop(
            TextureWorker,
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod arena_js_tests {
    use super::*;
    use afterglow_rpc::handle::LeaseState;
    use deno_core::{JsRuntime, RuntimeOptions};

    deno_core::extension!(arena_test_ext, ops = [op_afterglow_arena_view]);

    /// JS reads a native arena slot **in place** via a V8 external ArrayBuffer,
    /// with no copy into V8. The slot is leased to JS for the view's lifetime
    /// and released by the backing-store deleter when V8 GCs it.
    #[test]
    fn js_views_arena_slot_in_place_and_releases_on_gc() {
        let arena = Arena::new(0, 1, 64);
        let mut registry = ArenaRegistry::new();
        registry.register(0, arena.clone());
        let payload = b"hello arena";
        let handle = {
            let mut w = arena.acquire().unwrap();
            w.bytes()[..payload.len()].copy_from_slice(payload);
            w.handoff(payload.len() as u32)
        };
        // Slot is ReadLeased (handed off, not yet read).
        assert_eq!(
            arena.slot_state(handle.slot as usize),
            LeaseState::ReadLeased
        );

        let mut runtime = JsRuntime::new(RuntimeOptions {
            extensions: vec![arena_test_ext::init()],
            ..Default::default()
        });
        {
            let op_state = runtime.op_state();
            op_state.borrow_mut().put(registry);
        }

        let expected = payload
            .iter()
            .map(|b| b.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let script = format!(
            r#"
            const handle = {{region: {region}, slot: {slot}, length: {length}, generation: {generation}}};
            const view = Deno.core.ops.op_afterglow_arena_view(handle);
            const bytes = new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
            const expected = [{expected}];
            if (bytes.length !== expected.length) throw new Error('length mismatch');
            for (let i = 0; i < expected.length; i++) {{
              if (bytes[i] !== expected[i]) throw new Error('byte mismatch at ' + i);
            }}
            "#,
            region = handle.region,
            slot = handle.slot,
            length = handle.length,
            generation = handle.generation,
            expected = expected,
        );
        runtime
            .execute_script("<arena-view-test>", script)
            .expect("JS read the arena payload through the external-backed view");
        // The slot is leased to JS (Reading) while the view is alive.
        assert_eq!(arena.slot_state(handle.slot as usize), LeaseState::Reading);
        // Dropping the runtime tears down the isolate, which runs the external
        // backing store's deleter -> the slot is released back to the arena.
        drop(runtime);
        assert_eq!(arena.slot_state(handle.slot as usize), LeaseState::Free);

        // The slot can be re-acquired with an advanced generation.
        let w2 = arena.acquire().unwrap();
        assert_eq!(w2.slot(), handle.slot);
        assert_eq!(w2.generation(), handle.generation.wrapping_add(1));
    }
}
