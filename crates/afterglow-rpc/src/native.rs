//! Native worker transport: OS threads + a compact heap-backed ring buffer.
//!
//! [`spawn_worker_loop`] wires a worker impl into the request/response/event
//! ring buffers and runs the serve loop on a dedicated thread. The generated
//! `#[rpc]` `spawn_worker` calls it with a closure that delegates to the
//! trait's provided `serve` method.
//!
//! ## Backing
//!
//! [`RingStorage`] owns a single heap allocation per ring: a `RingAlloc`
//! (4-byte-aligned, `alloc_zeroed`, header written in place with `ptr::write`,
//! raw data only, `Drop` deallocs). Shared across the SPSC halves via `Arc`.
//! This is genuinely compact (1 byte per data byte, no per-element padding),
//! unlike a `Vec<UnsafeCell<u8>>` whose 4-byte alignment forces a 4-byte
//! stride. The web target later uses `RingHeader + UnsafeCell<[u8; N]>` and
//! views it through [`RingBuffer::from_header_data`].
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

use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::{ALIGN, HEADER_SIZE, RpcError, RpcResult};
use crate::{Response, RingBuffer, RingHeader, Transport, make_response, unwrap_response};

/// Default bounded-wait deadline for a single RPC response.
pub const RESPONSE_DEADLINE: Duration = Duration::from_secs(10);
/// Bounded wait for a worker to exit during [`WorkerTransport`] drop.
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(2);

// --- compact owned backing ------------------------------------------------

/// A 4-byte-aligned heap allocation owning a fully-constructed ring buffer
/// region: a [`RingHeader`] at `[0..HEADER_SIZE]` followed by `capacity` data
/// bytes. The header is written once in [`RingAlloc::new`]; the data area is
/// zeroed (== empty). Shared across the SPSC halves via `Arc`.
struct RingAlloc {
    /// Invariant: 4-byte-aligned, allocated with `layout`, points to
    /// `layout.size()` bytes; `[0..HEADER_SIZE]` is a constructed `RingHeader`,
    /// `[HEADER_SIZE..]` is the data area.
    ptr: NonNull<u8>,
    layout: Layout,
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
        let layout = Layout::from_size_align(total, ALIGN)
            .map_err(|_| RpcError::BadBacking(format!("bad layout for {total} bytes")))?;
        // SAFETY: `total >= HEADER_SIZE + 1` (>= 13) so non-zero; `ALIGN` is a
        // valid power-of-two alignment.
        let ptr = unsafe { alloc_zeroed(layout) };
        let ptr = NonNull::new(ptr)
            .ok_or_else(|| RpcError::BadBacking("ring allocation failed".into()))?;
        // Construct the header in place. The data area is already zeroed
        // (== empty ring). SAFETY: `ptr` is 4-aligned and valid for a write of
        // `RingHeader` (HEADER_SIZE bytes). `RingHeader` carries only `u32` +
        // atomics — no `Drop` glue beyond the allocation itself.
        unsafe {
            std::ptr::write(
                ptr.as_ptr() as *mut RingHeader,
                RingHeader::new(capacity as u32),
            );
        }
        Ok(Self { ptr, layout })
    }

    /// A `RingBuffer` view over this allocation's constructed region.
    fn view(&self) -> RingBuffer<'_> {
        let base = self.ptr.as_ptr();
        // SAFETY: `RingAlloc` invariant — `base` is 4-aligned, `[0..HEADER_SIZE]`
        // is a constructed `RingHeader` (written in `new`), and
        // `[HEADER_SIZE..layout.size()]` is `capacity` data bytes. Both valid
        // for `&self`'s lifetime. `RingHeader` is `repr(C)`, so the header sits
        // at offset 0 and the data area follows at `HEADER_SIZE`.
        unsafe {
            let header = &*(base as *const RingHeader);
            let data = base.add(HEADER_SIZE);
            RingBuffer::from_header_data(header, data, header.capacity() as usize)
                .expect("RingAlloc invariant: constructed ring buffer")
        }
    }
}

impl Drop for RingAlloc {
    fn drop(&mut self) {
        // SAFETY: `ptr` was allocated with `layout` in `new`; `Drop` runs once.
        // `RingHeader`/data are trivially droppable (no resources beyond the
        // allocation), so no per-field `Drop` is needed before dealloc.
        unsafe {
            dealloc(self.ptr.as_ptr(), self.layout);
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
    fn call(&self, _service: &str, method: u32, args: &[u8]) -> RpcResult<Vec<u8>> {
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

#[cfg(test)]
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
        let resp = t.call("svc", 7, &[1, 2, 3, 4]).unwrap();
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
        match t.call("svc", 0, &[]) {
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
        match t.call("svc", 0, &[]) {
            Err(RpcError::Server(_)) => {}
            other => panic!("expected server error, got {other:?}"),
        }
    }

    #[test]
    fn client_drop_joins_promptly() {
        let start = Instant::now();
        {
            let (t, _ev) = spawn_echo(1 << 16);
            let _ = t.call("svc", 0, &[9]).unwrap();
            // drop here
        }
        // Join + shutdown should be near-instant (well under the deadline).
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn dead_worker_call_returns_worker_dead() {
        let (t, _ev) = spawn_echo(1 << 16);
        // Warm up: confirm the worker is alive.
        assert_eq!(t.call("svc", 1, &[42]).unwrap(), vec![42]);
        // Send the shutdown control frame directly and let the worker exit.
        t.req.write(&[]).unwrap();
        t.wake();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !t.is_dead() && Instant::now() < deadline {
            thread::park_timeout(Duration::from_millis(1));
        }
        assert!(t.is_dead(), "worker should have exited");
        // A subsequent call reports WorkerDead rather than hanging.
        match t.call("svc", 2, &[]) {
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
        match t.call("svc", 1, b"first") {
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
        match t.call("svc", 2, b"second") {
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

        ta.call("svc", 0, &[]).unwrap();
        tb.call("svc", 0, &[]).unwrap();

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
        t.call("svc", 0, &[]).unwrap();
    }
}
