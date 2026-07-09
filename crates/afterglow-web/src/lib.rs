//! # afterglow-web
//!
//! Web target for afterglow-engine. Uses `SharedArrayBuffer`-backed shared
//! wasm memory for zero-copy ring buffers between Web Workers and the main
//! thread.
//!
//! ## Architecture
//!
//! On the web target, JS creates a `WebAssembly.Memory({ shared: true })`,
//! making the wasm linear memory a `SharedArrayBuffer`. Web Workers import
//! the same module + memory, so all code operates on shared memory.
//! `AtomicU32` with `Acquire/Release` ordering provides correct
//! synchronization across threads.
//!
//! ## No wasm-bindgen
//!
//! This crate uses `#[no_mangle]` exports only — no wasm-bindgen dependency.
//! JS creates the shared memory and instantiates the module manually. This
//! gives full control over the memory (must be `shared: true` for
//! `SharedArrayBuffer`).
//!
//! ## Exports
//!
//! - `get_ring_buffer_ptr() -> usize` — offset of the ring buffer in wasm memory
//! - `get_ring_buffer_size() -> usize` — total size (including 12-byte header)
//! - `ring_buffer_capacity() -> u32` — data area capacity
//! - `write_frame(ptr: *const u8, len: usize) -> i32` — write a frame (worker side)
//! - `read_frame(ptr: *mut u8, max_len: usize) -> i32` — read a frame (main thread)
//! - `has_data() -> i32` — non-blocking poll
//!
//! ## Requirements
//!
//! - Compile: `RUSTFLAGS='-C target-feature=+atomics,+bulk-memory,+mutable-globals' cargo build --target wasm32-unknown-unknown -Zbuild-std=core,alloc,std,panic_abort`
//! - Serve with COOP/COEP headers (for `SharedArrayBuffer` support)

use afterglow_rpc::RingBuffer;

/// The ring buffer lives in a static allocation in wasm linear memory.
/// When the `WebAssembly.Memory` is created with `shared: true`, this
/// static is automatically a `SharedArrayBuffer` — visible to all Web
/// Workers that share the memory.
const RING_BUFFER_SIZE: usize = 8 * 1024 * 1024; // 8 MiB
static mut RING_BUFFER: [u8; RING_BUFFER_SIZE] = [0; RING_BUFFER_SIZE];

/// Initialize the ring buffer header. Called once at startup.
#[unsafe(no_mangle)]
pub extern "C" fn init_ring_buffer() {
    unsafe {
        let buf = &mut RING_BUFFER[..];
        RingBuffer::init(buf);
    }
}

/// Get the offset of the ring buffer within wasm linear memory.
/// JS uses this to create `Uint8Array` / `DataView` views for direct access.
#[unsafe(no_mangle)]
pub extern "C" fn get_ring_buffer_ptr() -> usize {
    unsafe { (&raw const RING_BUFFER).cast::<u8>() as usize }
}

/// Total size of the ring buffer (including 12-byte header).
#[unsafe(no_mangle)]
pub extern "C" fn get_ring_buffer_size() -> usize {
    RING_BUFFER_SIZE
}

/// Data area capacity (excluding header).
#[unsafe(no_mangle)]
pub extern "C" fn ring_buffer_capacity() -> u32 {
    (RING_BUFFER_SIZE - 12) as u32
}

/// Write a frame to the ring buffer. Called from a Web Worker (physics).
/// Returns 0 on success, -1 if full.
#[unsafe(no_mangle)]
pub extern "C" fn write_frame(ptr: *const u8, len: usize) -> i32 {
    let data = unsafe { std::slice::from_raw_parts(ptr, len) };
    let rb = ring_buffer();
    match rb.write(data) {
        Ok(()) => 0,
        Err(afterglow_rpc::RpcError::BufferFull) => -1,
        Err(_) => -2,
    }
}

/// Read the next frame from the ring buffer into the provided buffer.
/// Called from the main thread (renderer).
/// Returns the number of bytes read, or -1 if empty.
#[unsafe(no_mangle)]
pub extern "C" fn read_frame(ptr: *mut u8, max_len: usize) -> i32 {
    let out = unsafe { std::slice::from_raw_parts_mut(ptr, max_len) };
    let rb = ring_buffer();
    match rb.read() {
        Ok(data) => {
            let n = data.len().min(max_len);
            out[..n].copy_from_slice(&data[..n]);
            n as i32
        }
        Err(_) => -1,
    }
}

/// Non-blocking: is there data to read?
#[unsafe(no_mangle)]
pub extern "C" fn has_data() -> i32 {
    let rb = ring_buffer();
    if rb.has_data() { 1 } else { 0 }
}

/// Read the capacity field from the actual ring buffer header in memory.
/// Used to verify that init_ring_buffer() actually wrote to the shared memory.
#[unsafe(no_mangle)]
pub extern "C" fn read_header_cap() -> u32 {
    unsafe {
        u32::from_le_bytes([
            RING_BUFFER[0], RING_BUFFER[1], RING_BUFFER[2], RING_BUFFER[3],
        ])
    }
}

/// Write a test marker to the ring buffer header and return it.
/// JS can verify it reads the same value via DataView.
#[unsafe(no_mangle)]
pub extern "C" fn write_test_marker() -> u32 {
    unsafe {
        RING_BUFFER[0] = 0xDE;
        RING_BUFFER[1] = 0xAD;
        RING_BUFFER[2] = 0xBE;
        RING_BUFFER[3] = 0xEF;
        u32::from_le_bytes([0xDE, 0xAD, 0xBE, 0xEF])
    }
}

fn ring_buffer<'a>() -> RingBuffer<'a> {
    unsafe { RingBuffer::new(&RING_BUFFER[..]) }
}
