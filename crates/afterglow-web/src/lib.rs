//! # afterglow-web
//!
//! Web target for afterglow-engine. Uses `SharedArrayBuffer`-backed shared
//! wasm memory for zero-copy ring buffers between Web Workers and the main
//! thread.
//!
//! Two ring buffers (same as native `RingBufferTransport`):
//! - **Request** (main→worker): main writes, worker reads.
//! - **Response** (worker→main): worker writes, main reads.
//!
//! Both use `afterglow_rpc::RingBuffer` on raw pointers + `AtomicU32`.
//!
//! ## Exports (all `#[no_mangle]`, no wasm-bindgen)
//!
//! Request buffer (main→worker):
//! - `write_frame(ptr, len)` — write a frame to the request buffer
//! - `read_frame(ptr, max_len)` — read a frame from the request buffer
//! - `has_data()` — poll the request buffer
//!
//! Response buffer (worker→main):
//! - `write_response(ptr, len)` — write a frame to the response buffer
//! - `read_response(ptr, max_len)` — read a frame from the response buffer
//! - `has_response()` — poll the response buffer
//!
//! Setup:
//! - `init_ring_buffers()` — initialize both headers
//! - `get_request_ptr()` / `get_response_ptr()` — offsets in wasm memory
//! - `get_buffer_size()` — total size per buffer

use afterglow_rpc::RingBuffer;

const BUFFER_SIZE: usize = 4 * 1024 * 1024; // 4 MiB per ring buffer

static mut REQUEST_BUFFER: [u8; BUFFER_SIZE] = [0; BUFFER_SIZE];
static mut RESPONSE_BUFFER: [u8; BUFFER_SIZE] = [0; BUFFER_SIZE];

#[unsafe(no_mangle)]
pub extern "C" fn init_ring_buffers() {
    unsafe {
        RingBuffer::init(&mut REQUEST_BUFFER[..]);
        RingBuffer::init(&mut RESPONSE_BUFFER[..]);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn get_request_ptr() -> usize {
    unsafe { (&raw const REQUEST_BUFFER).cast::<u8>() as usize }
}

#[unsafe(no_mangle)]
pub extern "C" fn get_response_ptr() -> usize {
    unsafe { (&raw const RESPONSE_BUFFER).cast::<u8>() as usize }
}

#[unsafe(no_mangle)]
pub extern "C" fn get_buffer_size() -> usize {
    BUFFER_SIZE
}

// --- Request buffer (main→worker) ---

#[unsafe(no_mangle)]
pub extern "C" fn write_frame(ptr: *const u8, len: usize) -> i32 {
    let data = unsafe { std::slice::from_raw_parts(ptr, len) };
    match request_rb().write(data) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn read_frame(ptr: *mut u8, max_len: usize) -> i32 {
    let out = unsafe { std::slice::from_raw_parts_mut(ptr, max_len) };
    match request_rb().read_into(out) {
        Ok(n) => n as i32,
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn has_data() -> i32 {
    if request_rb().has_data() { 1 } else { 0 }
}

// --- Response buffer (worker→main) ---

#[unsafe(no_mangle)]
pub extern "C" fn write_response(ptr: *const u8, len: usize) -> i32 {
    let data = unsafe { std::slice::from_raw_parts(ptr, len) };
    match response_rb().write(data) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn read_response(ptr: *mut u8, max_len: usize) -> i32 {
    let out = unsafe { std::slice::from_raw_parts_mut(ptr, max_len) };
    match response_rb().read_into(out) {
        Ok(n) => n as i32,
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn has_response() -> i32 {
    if response_rb().has_data() { 1 } else { 0 }
}

// --- Diagnostic ---

#[unsafe(no_mangle)]
pub extern "C" fn read_header_cap() -> u32 {
    request_rb().capacity()
}

#[unsafe(no_mangle)]
pub extern "C" fn write_test_marker() -> u32 {
    unsafe {
        REQUEST_BUFFER[0] = 0xDE;
        REQUEST_BUFFER[1] = 0xAD;
        REQUEST_BUFFER[2] = 0xBE;
        REQUEST_BUFFER[3] = 0xEF;
        u32::from_le_bytes([0xDE, 0xAD, 0xBE, 0xEF])
    }
}

fn request_rb<'a>() -> RingBuffer<'a> {
    unsafe { RingBuffer::new(&REQUEST_BUFFER[..]) }
}

fn response_rb<'a>() -> RingBuffer<'a> {
    unsafe { RingBuffer::new(&RESPONSE_BUFFER[..]) }
}
