//! # afterglow-rpc
//!
//! Ultra-low-latency communication over shared-memory ring buffers.
//!
//! Per worker, two ring buffers in shared memory (one per direction):
//! - **Request** (caller → worker): the caller writes framed requests.
//! - **Response** (worker → caller): the worker writes framed responses.
//!
//! Each ring buffer is a lock-free SPSC (single-producer single-consumer)
//! circular buffer with atomic read/write indices. Zero copies, zero
//! serialization overhead, zero IPC per call — just memory writes + atomics.
//!
//! - **Wire format**: [`postcard`] (compact, no schema bytes).
//! - **Framing**: `[len: u32 LE][payload]` per message in the ring buffer.
//! - **Transport**: [`RingBufferTransport`] reads/writes the ring buffer;
//!   the generated clients are transport-agnostic.
//! - **Native**: `afterglow_rpc::native::spawn_worker` creates ring buffers
//!   on heap memory (`Arc<Vec<u8>>`) and spawns an OS thread.
//! - **Web**: `afterglow-web` creates ring buffers on `SharedArrayBuffer`-
//!   backed wasm memory; workers are Web Workers sharing the same memory.
//!
//! Interfaces are defined once in Rust; the `afterglow-rpc-macros` `#[rpc]`
//! macro generates the server dispatch, the Rust client, and the schema.


use serde::{de::DeserializeOwned, Serialize};
use std::sync::atomic::{AtomicU32, Ordering};

#[cfg(not(target_arch = "wasm32"))]
pub mod native;

pub type RpcResult<T> = Result<T, RpcError>;

#[derive(Debug)]
pub enum RpcError {
    Codec(String),
    UnknownMethod,
    BufferFull,
    BufferEmpty,
    Transport(String),
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Codec(s) => write!(f, "rpc codec: {s}"),
            Self::UnknownMethod => write!(f, "rpc unknown method"),
            Self::BufferFull => write!(f, "rpc ring buffer full"),
            Self::BufferEmpty => write!(f, "rpc ring buffer empty"),
            Self::Transport(s) => write!(f, "rpc transport: {s}"),
        }
    }
}
impl std::error::Error for RpcError {}

// --- codec helpers (used by generated code) -------------------------------

pub fn encode<T: Serialize>(v: &T) -> RpcResult<Vec<u8>> {
    postcard::to_allocvec(v).map_err(|e| RpcError::Codec(e.to_string()))
}
pub fn decode<T: DeserializeOwned>(b: &[u8]) -> RpcResult<T> {
    postcard::from_bytes(b).map_err(|e| RpcError::Codec(e.to_string()))
}

// --- transport trait ------------------------------------------------------

/// The byte-pipe a generated client talks over. The ring buffer transport
/// implements this with zero-copy shared memory.
pub trait Transport {
    fn call(&self, service: &str, method: u32, args: &[u8]) -> RpcResult<Vec<u8>>;
}

// --- ring buffer ----------------------------------------------------------

/// A lock-free SPSC ring buffer in a shared byte slice.
///
/// Layout (in the backing buffer):
/// ```text
/// [capacity: u32][write_idx: AtomicU32][read_idx: AtomicU32][data...]
/// ```
///
/// The header (12 bytes) lives at the start of the buffer. `data` is a
/// circular buffer of `capacity` bytes. Each message is framed as
/// `[len: u32 LE][payload]`. Messages wrap around the end.
///
/// Producers call [`RingBuffer::write`], consumers call [`RingBuffer::read`]
/// or [`RingBuffer::read_into`] (no allocation). Both are lock-free
/// (atomic Acquire/Release).
pub struct RingBuffer<'a> {
    capacity: u32,
    write_idx: &'a AtomicU32,
    read_idx: &'a AtomicU32,
    data: &'a [u8],
}

const HEADER_SIZE: usize = 12; // capacity(4) + write_idx(4) + read_idx(4)

impl<'a> RingBuffer<'a> {
    /// Initialize a ring buffer in a fresh backing buffer.
    pub fn init(buf: &mut [u8]) {
        let cap = (buf.len() - HEADER_SIZE) as u32;
        buf[0..4].copy_from_slice(&cap.to_le_bytes());
        buf[4..8].copy_from_slice(&0u32.to_le_bytes()); // write_idx
        buf[8..12].copy_from_slice(&0u32.to_le_bytes()); // read_idx
    }

    /// Wrap an existing backing buffer as a ring buffer.
    pub fn new(buf: &'a [u8]) -> Self {
        let capacity = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        // SAFETY: the AtomicU32s are at fixed offsets in the shared buffer.
        // Both sides agree on the layout.
        let write_idx = unsafe { &*(buf[4..8].as_ptr() as *const AtomicU32) };
        let read_idx = unsafe { &*(buf[8..12].as_ptr() as *const AtomicU32) };
        let data = &buf[HEADER_SIZE..];
        Self { capacity, write_idx, read_idx, data }
    }

    /// Write a framed message `[len: u32][payload]` into the ring buffer.
    /// Returns `Err(BufferFull)` if there isn't enough space.
    pub fn write(&self, payload: &[u8]) -> RpcResult<()> {
        let frame_len = 4 + payload.len() as u32;
        let w = self.write_idx.load(Ordering::Relaxed);
        let r = self.read_idx.load(Ordering::Acquire);
        let used = w.wrapping_sub(r);
        let available = self.capacity.checked_sub(used).ok_or(RpcError::BufferFull)?;
        if frame_len > available {
            return Err(RpcError::BufferFull);
        }

        let data_len = self.capacity as usize;
        let offset = (w as usize) % data_len;
        let len_bytes = (payload.len() as u32).to_le_bytes();

        self.write_bytes(offset, &len_bytes);
        let payload_offset = (offset + 4) % data_len;
        self.write_bytes(payload_offset, payload);

        self.write_idx.store(w.wrapping_add(frame_len), Ordering::Release);
        Ok(())
    }

    /// Read the next framed message. Returns `Err(BufferEmpty)` if empty.
    /// Allocates a `Vec` — prefer [`read_into`](Self::read_into) for
    /// allocation-free reads (required for multi-instance wasm memory).
    pub fn read(&self) -> RpcResult<Vec<u8>> {
        let w = self.write_idx.load(Ordering::Acquire);
        let r = self.read_idx.load(Ordering::Relaxed);
        if w == r {
            return Err(RpcError::BufferEmpty);
        }

        let data_len = self.capacity as usize;
        let offset = (r as usize) % data_len;

        let mut len_bytes = [0u8; 4];
        self.read_bytes_into(offset, &mut len_bytes);
        let payload_len = u32::from_le_bytes(len_bytes) as usize;
        let frame_len = 4 + payload_len as u32;

        let payload_offset = (offset + 4) % data_len;
        let mut payload = vec![0u8; payload_len];
        self.read_bytes_into(payload_offset, &mut payload);

        self.read_idx.store(r.wrapping_add(frame_len), Ordering::Release);
        Ok(payload)
    }

    /// Read the next frame directly into `out`. Returns bytes written, or
    /// `Err(BufferEmpty)`. No allocation — safe for multi-instance shared
    /// wasm memory where allocators conflict.
    pub fn read_into(&self, out: &mut [u8]) -> RpcResult<usize> {
        let w = self.write_idx.load(Ordering::Acquire);
        let r = self.read_idx.load(Ordering::Relaxed);
        if w == r {
            return Err(RpcError::BufferEmpty);
        }

        let data_len = self.capacity as usize;
        let offset = (r as usize) % data_len;

        let mut len_bytes = [0u8; 4];
        self.read_bytes_into(offset, &mut len_bytes);
        let payload_len = u32::from_le_bytes(len_bytes) as usize;
        let frame_len = 4 + payload_len as u32;

        let n = payload_len.min(out.len());
        let payload_offset = (offset + 4) % data_len;
        self.read_bytes_into(payload_offset, &mut out[..n]);

        self.read_idx.store(r.wrapping_add(frame_len), Ordering::Release);
        Ok(n)
    }

    /// Non-blocking: is there data to read?
    pub fn has_data(&self) -> bool {
        self.write_idx.load(Ordering::Acquire) != self.read_idx.load(Ordering::Relaxed)
    }

    /// Data area capacity (excluding the 12-byte header).
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    // --- internal helpers ---

    fn write_bytes(&self, offset: usize, src: &[u8]) {
        let data_len = self.capacity as usize;
        if offset + src.len() <= data_len {
            unsafe {
                let dst = self.data.as_ptr().add(offset) as *mut u8;
                std::ptr::copy_nonoverlapping(src.as_ptr(), dst, src.len());
            }
        } else {
            let first = data_len - offset;
            unsafe {
                let dst = self.data.as_ptr().add(offset) as *mut u8;
                std::ptr::copy_nonoverlapping(src.as_ptr(), dst, first);
                let dst2 = self.data.as_ptr() as *mut u8;
                std::ptr::copy_nonoverlapping(src.as_ptr().add(first), dst2, src.len() - first);
            }
        }
    }

    fn read_bytes_into(&self, offset: usize, out: &mut [u8]) {
        let data_len = self.capacity as usize;
        let len = out.len();
        if offset + len <= data_len {
            unsafe {
                std::ptr::copy_nonoverlapping(self.data.as_ptr().add(offset), out.as_mut_ptr(), len);
            }
        } else {
            let first = data_len - offset;
            unsafe {
                std::ptr::copy_nonoverlapping(self.data.as_ptr().add(offset), out.as_mut_ptr(), first);
                std::ptr::copy_nonoverlapping(self.data.as_ptr(), out.as_mut_ptr().add(first), len - first);
            }
        }
    }
}

// --- ring buffer transport -----------------------------------------------

/// A transport backed by two ring buffers in shared memory:
/// - `request`: caller writes, worker reads.
/// - `response`: worker writes, caller reads.
///
/// `call()` writes a frame to the request buffer, then blocks reading the
/// response buffer until the worker responds.
pub struct RingBufferTransport<'a> {
    pub request: RingBuffer<'a>,
    pub response: RingBuffer<'a>,
}

impl<'a> Transport for RingBufferTransport<'a> {
    fn call(&self, _service: &str, method: u32, args: &[u8]) -> RpcResult<Vec<u8>> {
        let mut frame = Vec::with_capacity(4 + args.len());
        frame.extend_from_slice(&method.to_le_bytes());
        frame.extend_from_slice(args);
        self.request.write(&frame)?;
        loop {
            match self.response.read() {
                Ok(resp) => return Ok(resp),
                Err(RpcError::BufferEmpty) => std::hint::spin_loop(),
                Err(e) => return Err(e),
            }
        }
    }
}

// --- schema (for build-system codegen) ------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct RpcSchema {
    pub name: &'static str,
    pub methods: &'static [RpcMethod],
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct RpcMethod {
    pub id: u32,
    pub name: &'static str,
    pub params: &'static [(&'static str, &'static str)],
    pub returns: &'static str,
}

pub fn rust_type_to_ts(ty: &str) -> String {
    let ty = ty.trim();
    match ty {
        "bool" => "boolean".into(),
        "f32" | "f64" | "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "usize" | "isize" => "number".into(),
        "String" | "&str" | "str" => "string".into(),
        _ if ty.starts_with("Vec<u8>") || ty == "&[u8]" || ty == "[u8]" => "Uint8Array".into(),
        _ if ty.starts_with("Vec<") => {
            let inner = ty.trim_start_matches("Vec<").trim_end_matches('>');
            format!("{}[]", rust_type_to_ts(inner))
        }
        _ if ty.starts_with("Option<") => {
            let inner = ty.trim_start_matches("Option<").trim_end_matches('>');
            format!("{} | null", rust_type_to_ts(inner))
        }
        _ => ty.into(),
    }
}
