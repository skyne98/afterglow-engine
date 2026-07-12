//! Native worker transport: OS threads + a compact heap-backed ring buffer.
//!
//! [`spawn_worker_loop`] wires a worker impl into the request/response/event
//! ring buffers and runs the serve loop on a dedicated thread. The generated
//! `#[rpc]` `spawn_worker` calls it with a closure that delegates to the
//! trait's provided `serve` method.
//!
//! ## Backing
//!
//! [`RingStorage`] owns one four-byte-aligned boxed allocation per ring. It is
//! represented as initialized `u32` words solely to obtain alignment, with a
//! `RingHeader` constructed over the first 12 bytes and compact byte data after
//! it (at most three unused tail bytes). It is shared through `Arc`. The web
//! target uses `RingHeader + UnsafeCell<[u8; N]>` and views it through the same
//! [`RingBuffer::from_header_data`].
//!
//! ## Lifecycle
//!
//! - The client ([`WorkerTransport`]) owns the request producer, the response
//!   consumer, and the worker [`JoinHandle`]. There is **no side-channel
//!   `AtomicBool`** lifecycle: shutdown is a control frame on the request ring.
//! - [`WorkerTransport::call`] writes a request, wakes the worker, then
//!   bounded-waits for the response (park, not a tight spin). If the worker is
//!   dead ([`JoinHandle::is_finished`]) it returns [`RpcError::WorkerDead`];
//!   if no response arrives before the deadline, [`RpcError::Timeout`].
//! - Dropping the transport writes a shutdown control frame, wakes the worker,
//!   bounded-waits for it to exit, and joins it. No response write is ever
//!   silently dropped: an oversized response is replaced with a tiny error
//!   envelope.
//!
//! ## Events
//!
//! Events use a per-worker ring buffer (not `mpsc`). [`push_event`] writes to a
//! thread-local sink that [`run_worker_loop`] installs for the duration of the
//! loop, so multiple workers route events independently.
//!
//! ## Limitation
//!
//! Rust cannot forcibly stop a user `serve` method that never returns. If a
//! service method wedges, the worker cannot read the shutdown frame, so
//! [`WorkerTransport`]'s `Drop` detaches it after a bounded wait (the
//! `Arc`-backed ring buffers stay valid, so this leaks a thread without UB).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::Future;
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::pin::Pin;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::task::{Context, Poll, Waker};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::{ALIGN, HEADER_SIZE, RpcError, RpcResult};
use crate::{Response, RingBuffer, RingHeader, Transport, make_response, unwrap_response};

/// Default bounded-wait deadline for a single RPC response.
pub const RESPONSE_DEADLINE: Duration = Duration::from_secs(10);
/// Bounded wait for a worker to exit during [`WorkerTransport`] drop.
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(2);

// --- compact owned backing ------------------------------------------------

/// One aligned boxed allocation containing a constructed header followed by
/// `capacity` compact data bytes. `u32` words provide the required alignment;
/// the ring itself accesses the allocation as bytes.
struct RingAlloc {
    words: Box<[MaybeUninit<u32>]>,
    capacity: u32,
}

// SAFETY: the bytes are only mutated through the SPSC producer/consumer halves
// (one writer, one reader, coordinated by atomic indices on the header); no
// byte is mutated by two threads. The allocation is heap (not thread-local),
// so moving/sharing the owning `Arc` is sound. See `RingBuffer` safety docs.
unsafe impl Send for RingAlloc {}
unsafe impl Sync for RingAlloc {}

impl RingAlloc {
    fn new(capacity: usize) -> RpcResult<Self> {
        if capacity == 0 || capacity > u32::MAX as usize - HEADER_SIZE {
            return Err(RpcError::BadBacking(format!("invalid capacity {capacity}")));
        }
        let total = HEADER_SIZE + capacity;
        let word_count = total.div_ceil(ALIGN);
        let mut words = vec![MaybeUninit::new(0u32); word_count].into_boxed_slice();
        // SAFETY: boxed u32 storage is four-byte aligned and has at least
        // `HEADER_SIZE` bytes. The header has no drop glue.
        unsafe {
            std::ptr::write(
                words.as_mut_ptr().cast::<RingHeader>(),
                RingHeader::new(capacity as u32),
            );
        }
        Ok(Self {
            words,
            capacity: capacity as u32,
        })
    }

    /// A `RingBuffer` view over this allocation's constructed region.
    fn view(&self) -> RingBuffer<'_> {
        let base = self.words.as_ptr().cast::<u8>();
        // SAFETY: `new` constructed a header at the aligned allocation start;
        // at least `capacity` bytes follow it and remain valid for `&self`.
        unsafe {
            let header = &*base.cast::<RingHeader>();
            let data = base.add(HEADER_SIZE).cast_mut();
            RingBuffer::from_header_data(header, data, self.capacity as usize)
                .expect("RingAlloc invariant: constructed ring buffer")
        }
    }
}

// --- owned SPSC halves -----------------------------------------------------

/// Native owned ring buffer backing, sharable across threads via `Arc`.
/// Construct with [`RingStorage::new`]; split into a producer + consumer with
/// [`RingStorage::split`] (the normal cross-thread API — there is no public
/// combined read/write view).
pub struct RingStorage {
    alloc: Arc<RingAlloc>,
}

impl RingStorage {
    /// Allocate and initialize a ring buffer of `capacity` data bytes.
    pub fn new(capacity: usize) -> RpcResult<Self> {
        RingAlloc::new(capacity).map(|a| Self { alloc: Arc::new(a) })
    }

    /// Split into a (producer, consumer) pair sharing the same backing. Both
    /// halves are `Send + !Sync`: each lives on exactly one thread. Consumes
    /// `self`, so exactly one producer and one consumer are ever created from a
    /// given storage.
    pub fn split(self) -> (RingProducer, RingConsumer) {
        let alloc = self.alloc;
        (
            RingProducer {
                alloc: alloc.clone(),
                _not_sync: PhantomData,
            },
            RingConsumer {
                alloc,
                _not_sync: PhantomData,
            },
        )
    }
}

/// The write (producer) half of a native ring buffer. `Send + !Sync`.
pub struct RingProducer {
    alloc: Arc<RingAlloc>,
    _not_sync: PhantomData<Cell<()>>,
}

impl RingProducer {
    pub fn write(&self, payload: &[u8]) -> RpcResult<()> {
        self.alloc.view().write(payload)
    }
    pub fn capacity(&self) -> u32 {
        self.alloc.view().capacity()
    }
}

/// The read (consumer) half of a native ring buffer. `Send + !Sync`.
pub struct RingConsumer {
    alloc: Arc<RingAlloc>,
    _not_sync: PhantomData<Cell<()>>,
}

impl RingConsumer {
    pub fn read(&self) -> RpcResult<Vec<u8>> {
        self.alloc.view().read()
    }
    pub fn read_into(&self, out: &mut [u8]) -> RpcResult<usize> {
        self.alloc.view().read_into(out)
    }
    pub fn peek_len(&self) -> RpcResult<u32> {
        self.alloc.view().peek_len()
    }
    pub fn has_data(&self) -> bool {
        self.alloc.view().has_data()
    }
    pub fn capacity(&self) -> u32 {
        self.alloc.view().capacity()
    }
}

// --- worker transport (client side) ---------------------------------------

/// Client side of a native worker. Owns the request producer, response
/// consumer, and the worker [`JoinHandle`]. Dropping it requests shutdown (a
/// control frame on the request ring), wakes the worker, bounded-waits for it
/// to exit, and joins it.
pub struct WorkerTransport {
    req: RingProducer,
    resp: RingConsumer,
    handle: Option<JoinHandle<()>>,
    worker_thread: thread::Thread,
    /// Per-call response deadline. [`RESPONSE_DEADLINE`] in production; tests
    /// shrink it to exercise the timeout path without sleeping for 10 seconds.
    response_deadline: Duration,
    /// Latched `true` once a call times out. Thereafter every call fails
    /// immediately (as [`RpcError::WorkerDead`]) so a late-arriving response
    /// to the timed-out call can never be consumed as a later call's reply.
    ///
    /// `call`/`read_response_bounded` take `&self`, so the latch needs interior
    /// mutability; an atomic is used. `WorkerTransport` is `!Sync` (its ring
    /// halves are `!Sync`), so the flag is only touched from the owning client
    /// thread — the atomic keeps the access unambiguously sound regardless.
    poisoned: AtomicBool,
}

/// The worker-side halves handed to [`run_worker_loop`].
pub struct WorkerSide {
    pub req: RingConsumer,
    pub resp: RingProducer,
    pub events: RingProducer,
    /// Client thread to unpark after writing a response (low-latency wake).
    pub client_thread: thread::Thread,
}

impl WorkerTransport {
    /// Is the worker thread no longer running?
    fn is_dead(&self) -> bool {
        self.handle.as_ref().is_none_or(|h| h.is_finished())
    }

    /// Wake the worker thread after writing a request/shutdown frame.
    fn wake(&self) {
        self.worker_thread.unpark();
    }

    /// Bounded-wait for a response frame: park (not a tight spin) until the
    /// response arrives, the worker dies, or the deadline passes.
    fn read_response_bounded(&self) -> RpcResult<Vec<u8>> {
        let deadline = Instant::now() + self.response_deadline;
        loop {
            match self.resp.read() {
                Ok(bytes) => return Ok(bytes),
                Err(RpcError::BufferEmpty) => {}
                Err(e) => return Err(e),
            }
            if self.is_dead() {
                return Err(RpcError::WorkerDead);
            }
            if Instant::now() >= deadline {
                // The response may still arrive later and be mistaken for a
                // subsequent call's reply. Latch the transport poisoned so no
                // later `call` ever reads the response ring again.
                self.poisoned.store(true, Ordering::Release);
                return Err(RpcError::Timeout);
            }
            // The worker unparks us after writing a response; the short timeout
            // bounds the wait against a missed/raced unpark.
            thread::park_timeout(Duration::from_millis(1));
        }
    }
}

impl Transport for WorkerTransport {
    fn call(&self, method: u32, args: &[u8]) -> RpcResult<Vec<u8>> {
        // A previously timed-out call latched this transport poisoned: reject
        // immediately (as `WorkerDead`) rather than writing a new request and
        // possibly consuming the timed-out call's late response as our reply.
        if self.poisoned.load(Ordering::Acquire) || self.is_dead() {
            return Err(RpcError::WorkerDead);
        }
        // Frame: [method: u32 LE][args].
        let mut frame = Vec::with_capacity(4 + args.len());
        frame.extend_from_slice(&method.to_le_bytes());
        frame.extend_from_slice(args);
        self.req.write(&frame)?;
        self.wake();
        let bytes = self.read_response_bounded()?;
        unwrap_response(&bytes)
    }
}

impl Drop for WorkerTransport {
    fn drop(&mut self) {
        // Shutdown via the request ring (the communication mechanism, not a
        // side-channel flag): an empty request frame is the shutdown control
        // frame — a real request always carries ≥4 bytes for the method id.
        let _ = self.req.write(&[]);
        self.wake();
        if let Some(handle) = self.handle.take() {
            // Bounded-wait for the worker to exit, then join. A normal idle
            // worker reads the shutdown frame and exits in µs. If a user
            // `serve` method never returns, the worker cannot read the frame
            // and cannot be stopped (Rust has no thread cancellation); we
            // detach after the deadline rather than blocking Drop forever. The
            // `Arc`-backed ring buffers stay valid, so a wedged worker leaks a
            // thread without UB.
            let deadline = Instant::now() + SHUTDOWN_DEADLINE;
            while !handle.is_finished() && Instant::now() < deadline {
                thread::park_timeout(Duration::from_millis(1));
            }
            if handle.is_finished() {
                let _ = handle.join();
            } else {
                eprintln!(
                    "[afterglow] worker did not exit within {SHUTDOWN_DEADLINE:?}; \
                     detaching (a serve method may have wedged)"
                );
                // `handle` drops here, detaching the still-running thread.
            }
        }
    }
}

// --- events ---------------------------------------------------------------

/// Client-side drain for a worker's event stream (a ring buffer, not mpsc).
pub struct EventReceiver {
    cons: RingConsumer,
}

impl EventReceiver {
    pub fn try_recv(&self) -> Option<Vec<u8>> {
        self.cons.read().ok()
    }
    pub fn drain_into(&self, out: &mut Vec<Vec<u8>>) {
        while let Ok(ev) = self.cons.read() {
            out.push(ev);
        }
    }
    pub fn has_data(&self) -> bool {
        self.cons.has_data()
    }
}

thread_local! {
    /// Installed by `run_worker_loop` for the duration of the loop so that
    /// `push_event` writes to this worker's event ring buffer. Thread-local
    /// (`!Sync`), so multiple workers (on separate threads) route events
    /// independently.
    static EVENT_SINK: RefCell<Option<NonNull<RingProducer>>> = const { RefCell::new(None) };
}

/// Push an event onto the current worker's event ring buffer.
///
/// Returns `Err(BufferFull)` if the event ring is full (the worker cannot grow
/// it; the caller's `serve` method may drop, log, or propagate the error).
/// Returns `Ok(())` on success, or `Ok(())` (no-op) when called outside a
/// worker loop (no sink installed).
pub fn push_event(bytes: &[u8]) -> RpcResult<()> {
    EVENT_SINK.with(|c| match *c.borrow() {
        None => Ok(()),
        Some(ptr) => {
            // SAFETY: set by `run_worker_loop` on this thread; the
            // `RingProducer` lives for the whole loop and the sink is cleared
            // before the loop returns.
            let prod = unsafe { ptr.as_ref() };
            prod.write(bytes)
        }
    })
}

/// Install `events` as this thread's event sink, returning a guard that clears
/// it on drop (so the sink never outlives the producer).
fn install_event_sink(events: &RingProducer) -> EventSinkGuard<'_> {
    EVENT_SINK.with(|c| *c.borrow_mut() = Some(NonNull::from(events)));
    EventSinkGuard(PhantomData)
}

struct EventSinkGuard<'a>(PhantomData<&'a ()>);
impl Drop for EventSinkGuard<'_> {
    fn drop(&mut self) {
        EVENT_SINK.with(|c| *c.borrow_mut() = None);
    }
}

// --- worker loop ----------------------------------------------------------

/// Run a worker's serve loop on the current thread: read request frames,
/// dispatch via `serve`, write the [`Response`] envelope to the response ring,
/// and push events via the thread-local sink.
///
/// Shutdown is a control frame on the request ring: an **empty request frame**
/// (payload length 0) breaks the loop. A real request always carries ≥4 bytes
/// for the method id, so this never collides with user methods.
///
/// `serve` is `Fn(&mut S, u32, &[u8]) -> RpcResult<Vec<u8>>`; the `#[rpc]`
/// macro passes `|s, m, a| s.serve(m, a)` (the trait's provided dispatch).
pub fn run_worker_loop<S, F>(mut impl_: S, side: WorkerSide, serve: F)
where
    S: Send + 'static,
    F: Fn(&mut S, u32, &[u8]) -> RpcResult<Vec<u8>> + Send + 'static,
{
    let _sink = install_event_sink(&side.events);
    loop {
        match side.req.read() {
            Ok(frame) if frame.is_empty() => break, // shutdown control frame
            Ok(frame) => {
                if frame.len() < 4 {
                    // Malformed request: respond with an error envelope so the
                    // client does not hang, then continue.
                    let env =
                        make_response(0, Err(RpcError::CorruptFrame("frame too short".into())));
                    let _ = write_envelope(&side.resp, &env);
                    side.client_thread.unpark();
                    continue;
                }
                let method = u32::from_le_bytes(frame[0..4].try_into().unwrap());
                let args = &frame[4..];
                let res = serve(&mut impl_, method, args);
                let env = make_response(method, res);
                if let Err(e) = write_envelope(&side.resp, &env) {
                    // Response ring full or oversized. Try a tiny error
                    // envelope so the client gets an error instead of a hang.
                    let err = Response::Server {
                        method,
                        message: format!("response write failed: {e}"),
                    };
                    let _ = write_envelope(&side.resp, &err);
                }
                side.client_thread.unpark();
            }
            Err(RpcError::BufferEmpty) => {
                // Idle: park with a short timeout. The client unparks us after
                // writing a request (or shutdown frame). The timeout bounds
                // against a missed/raced unpark.
                thread::park_timeout(Duration::from_micros(500));
            }
            Err(e) => {
                eprintln!("[afterglow] worker ring error: {e}; exiting loop");
                break;
            }
        }
    }
}

/// Encode a [`Response`] and write it to the response ring.
fn write_envelope(resp: &RingProducer, env: &Response) -> RpcResult<()> {
    let bytes = crate::encode(env)?;
    resp.write(&bytes)
}

// --- spawn helper (used by the `#[rpc]` macro) ----------------------------

/// Spawn a worker thread running `serve` over fresh request/response/event
/// ring buffers of `capacity` data bytes each. Returns the client transport +
/// event receiver. The worker thread is joined when the transport is dropped.
pub fn spawn_worker_loop<S, F>(
    impl_: S,
    capacity: usize,
    serve: F,
) -> RpcResult<(WorkerTransport, EventReceiver)>
where
    S: Send + 'static,
    F: Fn(&mut S, u32, &[u8]) -> RpcResult<Vec<u8>> + Send + 'static,
{
    let req = RingStorage::new(capacity)?;
    let resp = RingStorage::new(capacity)?;
    let events = RingStorage::new(capacity)?;
    let (req_prod, req_cons) = req.split();
    let (resp_prod, resp_cons) = resp.split();
    let (ev_prod, ev_cons) = events.split();
    let client_thread = thread::current();
    let side = WorkerSide {
        req: req_cons,
        resp: resp_prod,
        events: ev_prod,
        client_thread,
    };
    let handle = thread::spawn(move || {
        run_worker_loop(impl_, side, serve);
    });
    // Wake the *worker* (not the caller) after writing a request: store the
    // worker thread handle, not `thread::current()` from here.
    let worker_thread = handle.thread().clone();
    let transport = WorkerTransport {
        req: req_prod,
        resp: resp_cons,
        handle: Some(handle),
        worker_thread,
        response_deadline: RESPONSE_DEADLINE,
        poisoned: AtomicBool::new(false),
    };
    Ok((transport, EventReceiver { cons: ev_cons }))
}

// --- async worker transport (poll model) ---------------------------------
//
// For `#[rpc]` traits with `async fn` methods. The worker runs an
// `async-executor::LocalExecutor` to drive async tasks; the client is non-blocking
// (`call_async` writes a request and returns a [`Oneshot`] future; `poll`
// drains completions and resolves pending futures). Multiple in-flight calls
// are supported — each gets a unique task_id for matching.
//
// Framing:
// - Request:  `[method: u32 LE][task_id: u64 LE][postcard args]`
// - Completion: `[task_id: u64 LE][Response envelope]` on the response ring
//
// The client's `poll()` drains the response ring and resolves pending
// oneshot receivers. The caller drives `poll()` each frame (the poll model):
// `loadAsset` returns a Future that resolves on a later `poll()`.

/// A boxed async serve future: `Pin<Box<dyn Future<Output = RpcResult<Vec<u8>>> + 'static>>`.
pub type ServeFuture = Pin<Box<dyn Future<Output = RpcResult<Vec<u8>>> + 'static>>;

/// A simple oneshot future: resolves when `poll()` delivers the result.
/// No waker registration — the caller's executor polls it, and `poll()` (the
/// transport's poll, called each frame) is what delivers the value.
pub struct Oneshot {
    inner: Arc<OneshotInner>,
}

struct OneshotInner {
    value: Mutex<Option<RpcResult<Vec<u8>>>>,
    waker: Mutex<Option<Waker>>,
}

impl Oneshot {
    fn pair() -> (OneshotSender, Oneshot) {
        let inner: Arc<OneshotInner> = Arc::new(OneshotInner {
            value: Mutex::new(None),
            waker: Mutex::new(None),
        });
        (
            OneshotSender {
                inner: inner.clone(),
            },
            Oneshot { inner },
        )
    }
}

impl Future for Oneshot {
    type Output = RpcResult<Vec<u8>>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut val = self.inner.value.lock().unwrap();
        if let Some(v) = val.take() {
            Poll::Ready(v)
        } else {
            *self.inner.waker.lock().unwrap() = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

/// The sender half of an [`Oneshot`]. Used by `AsyncWorkerTransport::poll` to
/// deliver completions.
pub struct OneshotSender {
    inner: Arc<OneshotInner>,
}

impl OneshotSender {
    fn send(self, value: RpcResult<Vec<u8>>) {
        *self.inner.value.lock().unwrap() = Some(value);
        if let Some(waker) = self.inner.waker.lock().unwrap().take() {
            waker.wake();
        }
    }
}

/// Client side of an async native worker. Non-blocking: `call_async` writes a
/// request and returns a [`Oneshot`] future; `poll` drains completions from the
/// response ring and resolves pending futures.
///
/// The caller must call `poll()` each frame (or on a timer) to drive completion.
/// `call_async` never blocks.
/// Client side of an async native worker. Non-blocking: `call_async` writes a
/// request and returns a [`Oneshot`] future; `poll` drains completions from the
/// response ring and resolves pending futures.
///
/// The caller must call `poll()` each frame (or on a timer) to drive completion.
/// `call_async` never blocks.
///
/// **Thread-safe:** the request/response ring halves are behind `Mutex`es so
/// multiple threads can share an `Arc<AsyncWorkerTransport>` and call
/// `call_async` / `poll` concurrently. The mutexes are held only for the
/// microsecond-level ring write/read — never during an `await`.
pub struct AsyncWorkerTransport {
    req: Mutex<RingProducer>,
    resp: Mutex<RingConsumer>,
    handle: Mutex<Option<JoinHandle<()>>>,
    worker_thread: thread::Thread,
    next_task_id: AtomicU64,
    pending: Mutex<HashMap<u64, OneshotSender>>,
    poisoned: AtomicBool,
}

impl AsyncWorkerTransport {
    /// Is the worker thread no longer running?
    fn is_dead(&self) -> bool {
        self.handle
            .lock()
            .unwrap()
            .as_ref()
            .is_none_or(|h| h.is_finished())
    }

    /// Non-blocking call: writes `[method][task_id][args]` to the request ring,
    /// registers a pending oneshot, and returns the [`Oneshot`] future. The
    /// caller `await`s the future; it resolves on a later `poll()`.
    ///
    /// Thread-safe: the request ring write is mutex-protected (microsecond-level).
    pub fn call_async(&self, method: u32, args: &[u8]) -> RpcResult<Oneshot> {
        if self.poisoned.load(Ordering::Acquire) || self.is_dead() {
            return Err(RpcError::WorkerDead);
        }
        let task_id = self.next_task_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = Oneshot::pair();
        self.pending.lock().unwrap().insert(task_id, tx);

        let mut frame = Vec::with_capacity(12 + args.len());
        frame.extend_from_slice(&method.to_le_bytes());
        frame.extend_from_slice(&task_id.to_le_bytes());
        frame.extend_from_slice(args);
        // Lock the request producer — multiple threads may write concurrently.
        match self.req.lock().unwrap().write(&frame) {
            Ok(()) => {
                self.worker_thread.unpark();
                Ok(rx)
            }
            Err(e) => {
                self.pending.lock().unwrap().remove(&task_id);
                Err(e)
            }
        }
    }

    /// Drain completions from the response ring and resolve pending futures.
    /// Call this each frame (or on a timer). Never blocks.
    ///
    /// Thread-safe: the response ring read is mutex-protected.
    pub fn poll(&self) {
        let resp = self.resp.lock().unwrap();
        while let Ok(completion) = resp.read() {
            if completion.len() < 8 {
                continue;
            }
            let task_id = u64::from_le_bytes(completion[0..8].try_into().unwrap());
            let response_bytes = completion[8..].to_vec();
            let result = unwrap_response(&response_bytes);
            if let Some(tx) = self.pending.lock().unwrap().remove(&task_id) {
                tx.send(result);
            }
        }
    }
}

impl Drop for AsyncWorkerTransport {
    fn drop(&mut self) {
        // Same shutdown as WorkerTransport: empty request frame.
        let _ = self.req.lock().unwrap().write(&[]);
        self.worker_thread.unpark();
        let handle = self.handle.lock().unwrap().take();
        if let Some(handle) = handle {
            let deadline = Instant::now() + SHUTDOWN_DEADLINE;
            while !handle.is_finished() && Instant::now() < deadline {
                thread::park_timeout(Duration::from_millis(1));
            }
            if handle.is_finished() {
                let _ = handle.join();
            }
        }
    }
}

/// The async serve function type: takes `&self` (the impl), method, args → a
/// boxed future producing `RpcResult<Vec<u8>>`. `&self` (not `&mut self`)
/// because multiple in-flight tasks can't each borrow `&mut`.

/// Spawn an async worker thread running `serve_async` over fresh
/// request/response/event ring buffers. The worker uses an
/// `async-executor::Executor` to drive async tasks. Returns the async client
/// transport + event receiver.
///
/// The worker thread:
/// 1. Reads request frames from the request ring.
/// 2. For each: spawns `serve_async(impl, method, args)` on the executor.
/// 3. When a task completes: writes `[task_id][Response]` to the response ring,
///    unparks the client.
/// 4. Ticks the executor when idle.
pub fn spawn_async_worker_loop<S, F>(
    impl_: S,
    capacity: usize,
    serve_async: F,
) -> RpcResult<(AsyncWorkerTransport, EventReceiver)>
where
    S: Send + 'static,
    F: Fn(&S, u32, &[u8]) -> ServeFuture + Send + 'static,
{
    let req = RingStorage::new(capacity)?;
    let resp = RingStorage::new(capacity)?;
    let events = RingStorage::new(capacity)?;
    let (req_prod, req_cons) = req.split();
    let (resp_prod, resp_cons) = resp.split();
    let (ev_prod, ev_cons) = events.split();
    let client_thread = thread::current();
    // Extract the response ring's Arc for cross-task writes (RingProducer is
    // !Sync, but Arc<RingAlloc> is Send+Sync — the executor writes through it).
    let resp_arc = resp_prod.alloc.clone();
    drop(resp_prod); // don't hold the !Sync producer; we write via the Arc
    let side = AsyncWorkerSide {
        req: req_cons,
        resp: resp_arc,
        events: ev_prod,
        client_thread,
    };
    let handle = thread::spawn(move || {
        run_async_worker_loop(impl_, side, serve_async);
    });
    let worker_thread = handle.thread().clone();
    let transport = AsyncWorkerTransport {
        req: Mutex::new(req_prod),
        resp: Mutex::new(resp_cons),
        handle: Mutex::new(Some(handle)),
        worker_thread,
        next_task_id: AtomicU64::new(0),
        pending: Mutex::new(HashMap::new()),
        poisoned: AtomicBool::new(false),
    };
    Ok((transport, EventReceiver { cons: ev_cons }))
}

/// Worker-side halves for the async loop.
struct AsyncWorkerSide {
    req: RingConsumer,
    resp: Arc<RingAlloc>,
    events: RingProducer,
    client_thread: thread::Thread,
}

/// Run an async worker's serve loop on the current thread.
///
/// Reads request frames `[method][task_id][args]`, spawns `serve_async` on
/// an `async-executor::Executor`, and writes completions `[task_id][Response]`
/// to the response ring when tasks complete. The executor is ticked on each
/// loop iteration.
pub fn run_async_worker_loop<S, F>(impl_: S, side: AsyncWorkerSide, serve_async: F)
where
    S: Send + 'static,
    F: Fn(&S, u32, &[u8]) -> ServeFuture + Send + 'static,
{
    let _sink = install_event_sink(&side.events);
    // Single-threaded executor (no Send requirement) — compiles on wasm too.
    let executor = async_executor::LocalExecutor::new();

    loop {
        // 1. Drain requests.
        match side.req.read() {
            Ok(frame) if frame.is_empty() => break,
            Ok(frame) => {
                if frame.len() < 12 {
                    continue;
                }
                let method = u32::from_le_bytes(frame[0..4].try_into().unwrap());
                let task_id = u64::from_le_bytes(frame[4..12].try_into().unwrap());
                let args = frame[12..].to_vec();
                let fut = serve_async(&impl_, method, &args);
                let resp_arc = side.resp.clone();
                let client_thread = side.client_thread.clone();
                executor
                    .spawn(async move {
                        let result = fut.await;
                        let env = make_response(method, result);
                        let env_bytes = crate::encode(&env).unwrap_or_default();
                        let mut completion = Vec::with_capacity(8 + env_bytes.len());
                        completion.extend_from_slice(&task_id.to_le_bytes());
                        completion.extend_from_slice(&env_bytes);
                        let _ = resp_arc.view().write(&completion);
                        client_thread.unpark();
                    })
                    .detach();
            }
            Err(RpcError::BufferEmpty) => {}
            Err(e) => {
                eprintln!("[afterglow] async worker ring error: {e}; exiting");
                break;
            }
        }
        // 2. Tick the executor (drive async tasks).
        executor.try_tick();
        // 3. Park briefly when idle.
        if !side.req.has_data() {
            thread::park_timeout(Duration::from_micros(500));
        }
    }
}
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::{RpcError, Transport, encode, make_response};

    /// A trivial serve fn: echoes the args back as the "payload".
    fn echo_serve<S>(_s: &mut S, _method: u32, args: &[u8]) -> RpcResult<Vec<u8>> {
        Ok(args.to_vec())
    }

    fn spawn_echo(capacity: usize) -> (WorkerTransport, EventReceiver) {
        spawn_worker_loop((), capacity, echo_serve).unwrap()
    }

    /// Bounded-wait read for cross-thread consumer tests (avoids a single
    /// `read()` racing the producer's write).
    fn bounded_read(cons: &RingConsumer, timeout: Duration) -> RpcResult<Vec<u8>> {
        let deadline = Instant::now() + timeout;
        loop {
            match cons.read() {
                Ok(v) => return Ok(v),
                Err(RpcError::BufferEmpty) => {}
                Err(e) => return Err(e),
            }
            if Instant::now() >= deadline {
                return Err(RpcError::Timeout);
            }
            thread::park_timeout(Duration::from_micros(100));
        }
    }

    #[test]
    fn split_roundtrip() {
        let s = RingStorage::new(256).unwrap();
        let (p, c) = s.split();
        assert_eq!(p.capacity(), 256);
        assert_eq!(c.capacity(), 256);
        assert!(!c.has_data());
        p.write(&[1, 2, 3]).unwrap();
        assert_eq!(c.read().unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn split_cross_thread_echo() {
        let s = RingStorage::new(1 << 16).unwrap();
        let (prod, cons) = s.split();
        let handle = thread::spawn(move || -> RpcResult<()> {
            let payload = bounded_read(&cons, Duration::from_secs(5))?;
            assert_eq!(payload, b"hello");
            Ok(())
        });
        prod.write(b"hello").unwrap();
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn happy_path_round_trip() {
        let (t, _ev) = spawn_echo(1 << 16);
        let resp = t.call(7, &[1, 2, 3, 4]).unwrap();
        assert_eq!(resp, vec![1, 2, 3, 4]);
    }

    #[test]
    fn unit_result_envelope() {
        // A method returning () -> postcard of () is 0 bytes; the envelope
        // still carries it and the client distinguishes Ok([]) from an error.
        let env = make_response(0, Ok(Vec::new()));
        let bytes = encode(&env).unwrap();
        let payload = unwrap_response(&bytes).unwrap();
        assert!(payload.is_empty());
    }

    #[test]
    fn server_error_envelope_preserved() {
        let (t, _ev) = spawn_worker_loop((), 1 << 16, |_: &mut (), _m, _a| {
            Err::<Vec<u8>, _>(RpcError::Server("boom".into()))
        })
        .unwrap();
        match t.call(0, &[]) {
            Err(RpcError::Server(m)) => assert_eq!(m, "boom"),
            other => panic!("expected Server(boom), got {other:?}"),
        }
    }

    #[test]
    fn oversized_response_returns_error_without_hang() {
        // Response larger than the response ring: the worker's envelope write
        // fails, it sends a tiny error envelope, the client gets an error.
        let (t, _ev) =
            spawn_worker_loop((), 256, |_: &mut (), _m, _a| Ok(vec![0u8; 10_000])).unwrap();
        match t.call(0, &[]) {
            Err(RpcError::Server(_)) => {}
            other => panic!("expected server error, got {other:?}"),
        }
    }

    #[test]
    fn client_drop_joins_promptly() {
        let start = Instant::now();
        {
            let (t, _ev) = spawn_echo(1 << 16);
            let _ = t.call(0, &[9]).unwrap();
            // drop here
        }
        // Join + shutdown should be near-instant (well under the deadline).
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn dead_worker_call_returns_worker_dead() {
        let (t, _ev) = spawn_echo(1 << 16);
        // Warm up: confirm the worker is alive.
        assert_eq!(t.call(1, &[42]).unwrap(), vec![42]);
        // Send the shutdown control frame directly and let the worker exit.
        t.req.write(&[]).unwrap();
        t.wake();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !t.is_dead() && Instant::now() < deadline {
            thread::park_timeout(Duration::from_millis(1));
        }
        assert!(t.is_dead(), "worker should have exited");
        // A subsequent call reports WorkerDead rather than hanging.
        match t.call(2, &[]) {
            Err(RpcError::WorkerDead) => {}
            other => panic!("expected WorkerDead, got {other:?}"),
        }
        // Drop normally (handle still present; worker already finished -> join
        // returns immediately).
    }

    #[test]
    fn timeout_poisons_transport_and_rejects_stale_response() {
        // Regression: a response that arrives *after* a call times out must
        // never be consumed as a later call's reply. The worker delays its
        // response past the client's short (test-injected) deadline, then
        // writes it late. The first call times out and latches the transport
        // poisoned; the second call must fail immediately without consuming
        // the stale response now sitting in the ring.
        let (mut t, _ev) = spawn_worker_loop((), 1 << 16, |_: &mut (), _m, _a| {
            thread::sleep(Duration::from_millis(150));
            Ok(b"late".to_vec())
        })
        .unwrap();
        // Inject a short deadline so the timeout path runs in tens of ms
        // rather than the production 10 seconds.
        t.response_deadline = Duration::from_millis(20);

        // First call: the worker is still sleeping, so the deadline elapses
        // and the call returns Timeout (latching the transport poisoned).
        match t.call(1, b"first") {
            Err(RpcError::Timeout) => {}
            other => panic!("expected Timeout, got {other:?}"),
        }

        // Wait for the worker to write the late response into the ring.
        let wait = Instant::now() + Duration::from_secs(2);
        while !t.resp.has_data() && Instant::now() < wait {
            thread::park_timeout(Duration::from_millis(1));
        }
        assert!(
            t.resp.has_data(),
            "stale response should be present in the ring"
        );

        // Second call: poisoned -> immediate WorkerDead, NOT the stale "late"
        // response that is sitting in the ring.
        match t.call(2, b"second") {
            Err(RpcError::WorkerDead) => {}
            other => panic!("expected WorkerDead (poisoned), got {other:?}"),
        }

        // The stale response is still unconsumed in the ring.
        assert!(
            t.resp.has_data(),
            "stale response must not have been consumed by the second call"
        );
    }

    #[test]
    fn two_workers_events_isolated() {
        // Worker A pushes an "A" event; worker B pushes a "B" event. Each
        // receiver must see only its own worker's events.
        let (ta, eva) = spawn_worker_loop((), 1 << 16, |_: &mut (), m, _a| {
            if m == 0 {
                push_event(b"A")?;
            }
            Ok(Vec::new())
        })
        .unwrap();
        let (tb, evb) = spawn_worker_loop((), 1 << 16, |_: &mut (), m, _a| {
            if m == 0 {
                push_event(b"B")?;
            }
            Ok(Vec::new())
        })
        .unwrap();

        ta.call(0, &[]).unwrap();
        tb.call(0, &[]).unwrap();

        let mut a_events = Vec::new();
        eva.drain_into(&mut a_events);
        let mut b_events = Vec::new();
        evb.drain_into(&mut b_events);

        assert!(
            a_events.iter().all(|e| e == b"A"),
            "A events leaked: {a_events:?}"
        );
        assert!(
            b_events.iter().all(|e| e == b"B"),
            "B events leaked: {b_events:?}"
        );
        assert!(!a_events.is_empty());
        assert!(!b_events.is_empty());
    }

    #[test]
    fn push_event_full_returns_err() {
        // Event ring cap 32; a 40-byte event (frame 44 > 32) cannot fit.
        let (t, _ev) = spawn_worker_loop((), 32, |_: &mut (), m, _a| {
            if m == 0 {
                match push_event(&[0u8; 40]) {
                    Err(RpcError::BufferFull) => {}
                    other => panic!("expected BufferFull, got {other:?}"),
                }
            }
            Ok(Vec::new())
        })
        .unwrap();
        t.call(0, &[]).unwrap();
    }
}
