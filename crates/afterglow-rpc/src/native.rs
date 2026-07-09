//! Native worker transport: OS threads + ring buffers in shared memory.
//!
//! `spawn_worker` (generated per service by `#[rpc]`) creates two ring buffers
//! (backed by heap-allocated `Vec<u8>` for same-process), spawns the worker
//! thread, and returns a client + an event receiver.
//!
//! For the web target, see `afterglow-web` (SharedArrayBuffer-backed).

use std::sync::mpsc::{Receiver, Sender};

use crate::{RingBuffer, RingBufferTransport, Transport, RpcResult, RpcError};

/// Client side of a native worker ring buffer transport.
pub struct WorkerTransport {
    // Keep the backing buffers alive for the RingBuffer's lifetime.
    _req_buf: std::sync::Arc<Vec<u8>>,
    _resp_buf: std::sync::Arc<Vec<u8>>,
    // The transport (borrows the bufs). We store it as a raw pointer + manually
    // manage the lifetime via the Arc above.
    transport_ptr: *const RingBufferTransport<'static>,
}

// SAFETY: the transport is only accessed from the thread that owns WorkerTransport.
// The backing buffers live as long as the Arcs.
unsafe impl Send for WorkerTransport {}
unsafe impl Sync for WorkerTransport {}

impl WorkerTransport {
    /// Create a pair of (caller, worker) ring buffer transports.
    /// Returns the caller transport + the worker's request/response buffers.
    pub fn new_pair(capacity: usize) -> (WorkerTransport, WorkerBuffers) {
        let mut req_buf = vec![0u8; capacity + 12];
        let mut resp_buf = vec![0u8; capacity + 12];
        RingBuffer::init(&mut req_buf);
        RingBuffer::init(&mut resp_buf);

        let req_arc = std::sync::Arc::new(req_buf);
        let resp_arc = std::sync::Arc::new(resp_buf);

        // Create the transport borrowing from the Arcs.
        let request = RingBuffer::new(&req_arc[..]);
        let response = RingBuffer::new(&resp_arc[..]);
        let transport = Box::new(RingBufferTransport { request, response });
        let transport_ptr = Box::into_raw(transport) as *const RingBufferTransport<'static>;

        // Worker gets clones of the same Arcs (same memory).
        let worker = WorkerBuffers {
            req_buf: req_arc.clone(),
            resp_buf: resp_arc.clone(),
        };

        (WorkerTransport {
            _req_buf: req_arc,
            _resp_buf: resp_arc,
            transport_ptr,
        }, worker)
    }

    fn transport(&self) -> &RingBufferTransport<'static> {
        unsafe { &*self.transport_ptr }
    }
}

impl Transport for WorkerTransport {
    fn call(&self, _service: &str, method: u32, args: &[u8]) -> RpcResult<Vec<u8>> {
        self.transport().call(_service, method, args)
    }
}

impl Drop for WorkerTransport {
    fn drop(&mut self) {
        unsafe { drop(Box::from_raw(self.transport_ptr as *mut RingBufferTransport<'static>)) }
    }
}

/// The worker side's ring buffer handles (shared memory with the caller).
pub struct WorkerBuffers {
    pub req_buf: std::sync::Arc<Vec<u8>>,
    pub resp_buf: std::sync::Arc<Vec<u8>>,
}

impl WorkerBuffers {
    /// Create the worker's transport (reads requests, writes responses).
    pub fn transport(&self) -> RingBufferTransport<'_> {
        let request = RingBuffer::new(&self.req_buf[..]);
        let response = RingBuffer::new(&self.resp_buf[..]);
        RingBufferTransport { request, response }
    }
}

/// Worker→main event stream (drained each frame).
pub struct EventReceiver {
    pub rx: Receiver<Vec<u8>>,
}
impl EventReceiver {
    pub fn try_recv(&self) -> Option<Vec<u8>> { self.rx.try_recv().ok() }
    pub fn drain_into(&self, out: &mut Vec<Vec<u8>>) {
        while let Ok(ev) = self.rx.try_recv() { out.push(ev); }
    }
}

static EVENT_TX: std::sync::Mutex<Option<Sender<Vec<u8>>>> = std::sync::Mutex::new(None);

pub fn set_event_sender(tx: Sender<Vec<u8>>) {
    *EVENT_TX.lock().expect("event tx lock") = Some(tx);
}

pub fn push_event(bytes: Vec<u8>) {
    if let Some(tx) = EVENT_TX.lock().expect("event tx lock").as_ref() {
        let _ = tx.send(bytes);
    }
}

/// Run a worker's serve loop on the current thread, reading from the ring
/// buffer and writing responses. The `serve` function is generated per-service
/// by the `#[rpc]` macro.
pub fn run_worker_loop<S, Serve>(mut impl_: S, bufs: WorkerBuffers, event_tx: Sender<Vec<u8>>, serve: Serve)
where
    S: Send + 'static,
    Serve: Fn(&mut S, u32, &[u8]) -> RpcResult<Vec<u8>> + Send + 'static,
{
    set_event_sender(event_tx);
    let transport = bufs.transport();
    // The worker reads from the REQUEST ring buffer (caller wrote there),
    // and writes to the RESPONSE ring buffer.
    // Note: the transport's `request` is the caller's write side; the worker
    // reads from it. The transport's `response` is the worker's write side.
    loop {
        match transport.request.read() {
            Ok(frame) => {
                if frame.len() < 4 { continue; }
                let method = u32::from_le_bytes(frame[0..4].try_into().unwrap());
                let args = &frame[4..];
                let resp = serve(&mut impl_, method, args);
                let resp_bytes = match resp {
                    Ok(b) => b,
                    Err(e) => format!("error: {e}").into_bytes(),
                };
                let _ = transport.response.write(&resp_bytes);
            }
            Err(RpcError::BufferEmpty) => {
                std::hint::spin_loop();
            }
            Err(e) => {
                eprintln!("[afterglow] worker ring buffer error: {e}");
                break;
            }
        }
    }
}
