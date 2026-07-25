//! Web (wasm32) ABI for afterglow comms.
//!
//! Two concerns live here, both `#[cfg(target_arch = "wasm32")]` only:
//!
//! - **Main-thread transport:** the SPSC request/response rings + scratch live
//!   in `SharedArrayBuffer`-backed wasm memory, built on
//!   [`crate::RingBuffer`] over `RingHeader + UnsafeCell<[u8; N]>` statics.
//!   `#[no_mangle] extern "C"` exports hand their addresses + a framed
//!   write/read API to JS. No `wasm-bindgen`; JS owns the memory and wakes the
//!   worker with a payload-free `postMessage` (the ring is the transport).
//! - **Service-side ABI:** [`Scratch`] + [`write_response`] helpers used by the
//!   `#[rpc]` macro's thin worker wasm exports.
//!
//! On native (`rlib`) this module is absent — the native transport lives in
//! [`crate::native`].
//!
//! Build (`.cargo/config.toml` applies `--import-memory` + `--shared-memory`):
//! `cargo build -p afterglow-rpc --target wasm32-unknown-unknown \
//!   -Zbuild-std=core,alloc,std,panic_abort --profile wasm-dev`

#![cfg(target_arch = "wasm32")]

use std::cell::UnsafeCell;

use crate::{RingBuffer, RingHeader, RpcError, RpcResult, Response, encode, make_response};

// === main-thread transport ==============================================

// JS-provided wake import: called after every successful request write so the
// worker wakes immediately. Wake only — the ring is the sole payload transport.
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn notify_worker();
}

/// Wake the worker after a successful request write.
#[inline]
fn wake_worker() {
    // SAFETY: JS provides `notify_worker` from the `env` module.
    unsafe {
        notify_worker();
    }
}

/// Data bytes per ring. 1 MiB matches the generated worker scratch and bounds
/// the static footprint (the prior 4 MiB was unused headroom).
const BUFFER_SIZE: usize = 1 << 20;
/// Main-thread scratch (build requests / read responses). One in-flight call,
/// so a single region sized to the ring capacity suffices.
const SCRATCH_SIZE: usize = 1 << 20;

/// 4-byte-aligned `RingHeader + UnsafeCell<[u8; N]>` storage. The header
/// carries capacity + atomic indices; the data area is interior-mutable for the
/// SPSC producer/consumer, mutated only via raw pointers (never `&[u8]`
/// aliasing between the two halves).
#[repr(C, align(4))]
struct StaticRing<const N: usize> {
    header: RingHeader,
    data: UnsafeCell<[u8; N]>,
}
// SAFETY: indices are atomic; the data area is touched only via raw pointers
// under the SPSC contract (the worker accesses the same bytes through JS, never
// through Rust references). See `RingBuffer`.
unsafe impl<const N: usize> Sync for StaticRing<N> {}

static REQUEST_BUFFER: StaticRing<BUFFER_SIZE> = StaticRing {
    header: RingHeader::new(BUFFER_SIZE as u32),
    data: UnsafeCell::new([0; BUFFER_SIZE]),
};
static RESPONSE_BUFFER: StaticRing<BUFFER_SIZE> = StaticRing {
    header: RingHeader::new(BUFFER_SIZE as u32),
    data: UnsafeCell::new([0; BUFFER_SIZE]),
};

#[repr(C, align(4))]
struct MainScratch(UnsafeCell<[u8; SCRATCH_SIZE]>);
// SAFETY: scratch is mutated only by the main-thread wasm via raw pointers
// (single Rust accessor; the worker never touches it).
unsafe impl Sync for MainScratch {}
static SCRATCH: MainScratch = MainScratch(UnsafeCell::new([0; SCRATCH_SIZE]));

/// Borrowed view over a static ring.
///
/// # Safety (internal)
/// `StaticRing` is `repr(C, align(4))` with a `RingHeader` constructed via
/// `RingHeader::new`; the `UnsafeCell` data area is N bytes immediately after
/// the 12-byte header. The SPSC contract is upheld by callers.
fn rb<'a, const N: usize>(ring: &'a StaticRing<N>) -> RingBuffer<'a> {
    unsafe {
        RingBuffer::from_header_data(&ring.header, ring.data.get().cast::<u8>(), N)
            .expect("static ring invariant: header capacity == N")
    }
}

/// Map a `read_into` result to the stable small-integer error codes used by JS:
/// `>=0` payload length, `-1` empty, `-2` too small (frame left for retry),
/// `-3` corrupt.
fn read_ring<const N: usize>(ring: &StaticRing<N>, out: &mut [u8]) -> i32 {
    match rb(ring).read_into(out) {
        Ok(n) => n as i32,
        Err(RpcError::BufferEmpty) => -1,
        Err(RpcError::BufferTooSmall { .. }) => -2,
        Err(_) => -3,
    }
}

/// Reset both rings to empty. Call once at startup. Idempotent.
#[unsafe(no_mangle)]
pub extern "C" fn init_ring_buffers() {
    REQUEST_BUFFER.header.reset();
    RESPONSE_BUFFER.header.reset();
}

/// Offset of the request ring in wasm linear memory.
#[unsafe(no_mangle)]
pub extern "C" fn get_request_ptr() -> usize {
    (&raw const REQUEST_BUFFER).cast::<u8>() as usize
}

/// Offset of the response ring in wasm linear memory.
#[unsafe(no_mangle)]
pub extern "C" fn get_response_ptr() -> usize {
    (&raw const RESPONSE_BUFFER).cast::<u8>() as usize
}

/// Total size of each ring (header + data).
#[unsafe(no_mangle)]
pub extern "C" fn get_buffer_size() -> usize {
    crate::HEADER_SIZE + BUFFER_SIZE
}

/// Offset of the main-thread scratch buffer (for building requests / reading
/// responses). JS must use this rather than guessing a free address.
#[unsafe(no_mangle)]
pub extern "C" fn get_scratch_ptr() -> usize {
    SCRATCH.0.get().cast::<u8>() as usize
}

/// Length of the main-thread scratch buffer.
#[unsafe(no_mangle)]
pub extern "C" fn get_scratch_size() -> usize {
    SCRATCH_SIZE
}

/// Write a request frame `[len:u32][payload]` to the request ring. On success
/// wakes the worker (wake only). Returns `0` ok, `-1` full, `-3` corrupt.
///
/// # Safety
/// `ptr` must point to `len` readable bytes in wasm linear memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn write_frame(ptr: *const u8, len: usize) -> i32 {
    // SAFETY: caller upholds `ptr` validity (see # Safety).
    let data = unsafe { std::slice::from_raw_parts(ptr, len) };
    match rb(&REQUEST_BUFFER).write(data) {
        Ok(()) => {
            wake_worker();
            0
        }
        Err(RpcError::BufferFull) => -1,
        Err(_) => -3,
    }
}

/// Read one response frame into `out`. Returns the payload length (>=0), `-1` if
/// empty, `-2` if `max_len` is too small (frame left for retry), `-3` corrupt.
///
/// # Safety
/// `ptr` must point to `max_len` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn read_response(ptr: *mut u8, max_len: usize) -> i32 {
    // SAFETY: caller upholds `ptr` validity (see # Safety).
    let out = unsafe { std::slice::from_raw_parts_mut(ptr, max_len) };
    read_ring(&RESPONSE_BUFFER, out)
}

// === service-side ABI (used by `#[rpc]` worker exports) =================

/// Fixed scratch storage whose address is exported to the JS worker.
#[repr(C, align(4))]
pub struct Scratch<const N: usize>(UnsafeCell<[u8; N]>);
// SAFETY: one worker instance accesses each scratch region synchronously.
unsafe impl<const N: usize> Sync for Scratch<N> {}

impl<const N: usize> Default for Scratch<N> {
    fn default() -> Self {
        Self(UnsafeCell::new([0; N]))
    }
}

impl<const N: usize> Scratch<N> {
    pub const fn new() -> Self {
        Self(UnsafeCell::new([0; N]))
    }
    pub fn ptr(&self) -> usize {
        self.0.get().cast::<u8>() as usize
    }
    pub const fn size(&self) -> usize {
        N
    }
}

/// Encode a service result as a response envelope into `out`. Oversized
/// responses are replaced by a compact error envelope.
pub fn write_response(method: u32, result: RpcResult<Vec<u8>>, out: &mut [u8]) -> i32 {
    let env = make_response(method, result);
    let bytes = match encode(&env) {
        Ok(bytes) if bytes.len() <= out.len() => bytes,
        _ => match encode(&Response::Server {
            method,
            message: "response too large".into(),
        }) {
            Ok(bytes) if bytes.len() <= out.len() => bytes,
            _ => return -1,
        },
    };
    out[..bytes.len()].copy_from_slice(&bytes);
    bytes.len() as i32
}
