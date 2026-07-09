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
