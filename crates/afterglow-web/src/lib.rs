//! # afterglow-web
//!
//! Web target for afterglow-engine. Uses `SharedArrayBuffer`-backed shared
//! wasm memory for zero-copy ring buffers between Web Workers and the main
//! thread.
//!
//! Two ring buffers (same layout as native `RingBufferTransport`):
//! - **Request** (main→worker): main writes, worker reads.
//! - **Response** (worker→main): worker writes, main reads.
//!
//! Both use `afterglow_rpc::RingBuffer` on raw pointers + `AtomicU32`.
//! No wasm-bindgen — `#[no_mangle]` exports only, JS controls the memory.
//!
//! ## Build
//!
//! ```sh
//! cargo build -p afterglow-web \
//!   --target wasm32-unknown-unknown \
//!   -Zbuild-std=core,alloc,std,panic_abort \
//!   --profile wasm-dev
//! ```
//!
//! The `.cargo/config.toml` at the workspace root applies `--import-memory`
//! + `--shared-memory` so the module uses JS-provided shared memory.

use afterglow_rpc::RingBuffer;

const BUFFER_SIZE: usize = 4 * 1024 * 1024; // 4 MiB per ring buffer

static mut REQUEST_BUFFER: [u8; BUFFER_SIZE] = [0; BUFFER_SIZE];
static mut RESPONSE_BUFFER: [u8; BUFFER_SIZE] = [0; BUFFER_SIZE];

/// Initialize both ring buffer headers. Call once at startup.
#[unsafe(no_mangle)]
pub extern "C" fn init_ring_buffers() {
    // SAFETY: called once before any concurrent access.
    unsafe {
        RingBuffer::init(&mut REQUEST_BUFFER[..]);
        RingBuffer::init(&mut RESPONSE_BUFFER[..]);
    }
}

/// Offset of the request ring buffer in wasm linear memory.
#[unsafe(no_mangle)]
pub extern "C" fn get_request_ptr() -> usize {
    (&raw const REQUEST_BUFFER).cast::<u8>() as usize
}

/// Offset of the response ring buffer in wasm linear memory.
#[unsafe(no_mangle)]
pub extern "C" fn get_response_ptr() -> usize {
    (&raw const RESPONSE_BUFFER).cast::<u8>() as usize
}

/// Total size of each ring buffer (including 12-byte header).
#[unsafe(no_mangle)]
pub extern "C" fn get_buffer_size() -> usize {
    BUFFER_SIZE
}

// --- Request buffer (main→worker) ---

/// Write a frame to the request ring buffer. Returns 0 on success, -1 if full.
#[unsafe(no_mangle)]
pub extern "C" fn write_frame(ptr: *const u8, len: usize) -> i32 {
    // SAFETY: caller provides a valid pointer + length within wasm memory.
    let data = unsafe { std::slice::from_raw_parts(ptr, len) };
    match request_rb().write(data) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// Read a frame from the request ring buffer into `ptr`. Returns bytes read
/// or -1 if empty. No allocation.
#[unsafe(no_mangle)]
pub extern "C" fn read_frame(ptr: *mut u8, max_len: usize) -> i32 {
    let out = unsafe { std::slice::from_raw_parts_mut(ptr, max_len) };
    match request_rb().read_into(out) {
        Ok(n) => n as i32,
        Err(_) => -1,
    }
}

/// Non-blocking: does the request buffer have data?
#[unsafe(no_mangle)]
pub extern "C" fn has_data() -> i32 {
    if request_rb().has_data() { 1 } else { 0 }
}

// --- Response buffer (worker→main) ---

/// Write a frame to the response ring buffer. Returns 0 on success, -1 if full.
#[unsafe(no_mangle)]
pub extern "C" fn write_response(ptr: *const u8, len: usize) -> i32 {
    let data = unsafe { std::slice::from_raw_parts(ptr, len) };
    match response_rb().write(data) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// Read a frame from the response ring buffer into `ptr`. Returns bytes read
/// or -1 if empty. No allocation.
#[unsafe(no_mangle)]
pub extern "C" fn read_response(ptr: *mut u8, max_len: usize) -> i32 {
    let out = unsafe { std::slice::from_raw_parts_mut(ptr, max_len) };
    match response_rb().read_into(out) {
        Ok(n) => n as i32,
        Err(_) => -1,
    }
}

/// Non-blocking: does the response buffer have data?
#[unsafe(no_mangle)]
pub extern "C" fn has_response() -> i32 {
    if response_rb().has_data() { 1 } else { 0 }
}

fn request_rb<'a>() -> RingBuffer<'a> {
    // SAFETY: REQUEST_BUFFER is a static; the RingBuffer borrows it with an
    // extended lifetime. The static lives for the program's duration.
    unsafe { RingBuffer::new(&REQUEST_BUFFER[..]) }
}

fn response_rb<'a>() -> RingBuffer<'a> {
    unsafe { RingBuffer::new(&RESPONSE_BUFFER[..]) }
}

// --- WebTransport: Transport impl using the ring buffer statics ----------

/// Scratch buffers for encoding requests and decoding responses.
/// In wasm linear memory — no allocation per call.
const SCRATCH_SIZE: usize = 1024 * 1024; // 1 MiB
static mut REQUEST_SCRATCH: [u8; SCRATCH_SIZE] = [0; SCRATCH_SIZE];
static mut RESPONSE_SCRATCH: [u8; SCRATCH_SIZE] = [0; SCRATCH_SIZE];

/// A `Transport` that reads/writes the ring buffer statics in shared wasm
/// memory. Used by the generated client on the main thread (web target).
///
/// Flow: encode args → write_frame (ring buffer + Atomics.notify) →
///       spin on has_response → read_response → decode.
///
/// The worker (Web Worker, own wasm memory) reads the request via JS,
/// calls `wasm_serve_frame`, and writes the response back.
pub struct WebTransport;

impl afterglow_rpc::Transport for WebTransport {
    fn call(&self, _service: &str, method: u32, args: &[u8]) -> afterglow_rpc::RpcResult<Vec<u8>> {
        // Build frame: [method:u32][args]
        let frame_len = 4 + args.len();
        // SAFETY: REQUEST_SCRATCH is a static; we have exclusive access
        // (single-threaded main thread).
        let scratch = unsafe { &mut REQUEST_SCRATCH[..] };
        if frame_len > scratch.len() {
            return Err(afterglow_rpc::RpcError::Transport("frame too large".into()));
        }
        scratch[..4].copy_from_slice(&method.to_le_bytes());
        scratch[4..frame_len].copy_from_slice(args);

        // Write to request ring buffer (also notifies the worker via
        // AtomicU32::notify inside RingBuffer::write)
        request_rb().write(&scratch[..frame_len])?;

        // Wait for response (spin — main thread can't Atomics.wait)
        let resp = loop {
            match response_rb().read_into(unsafe { &mut RESPONSE_SCRATCH[..] }) {
                Ok(n) => break unsafe { RESPONSE_SCRATCH[..n].to_vec() },
                Err(afterglow_rpc::RpcError::BufferEmpty) => {
                    // Yield to the event loop to avoid blocking the page
                    // (in practice, the response arrives within microseconds)
                    // TODO: use Atomics.wait with timeout when available on main thread
                    std::hint::spin_loop();
                }
                Err(e) => return Err(e),
            }
        };

        Ok(resp)
    }
}
