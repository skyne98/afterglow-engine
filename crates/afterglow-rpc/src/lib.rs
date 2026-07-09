//! # afterglow-rpc
//!
//! Ultra-low-latency RPC over shared-memory ring buffers.
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
//! - **Setup**: the shared memory handle is transferred once via `postMessage`
//!   (web: `SharedArrayBuffer`; native CEF: `CefSharedMemoryRegion`). After
//!   that, all comms go through the ring buffer — no `postMessage` per call.
//!
//! Interfaces are defined once in Rust; the `afterglow-rpc-macros` `#[rpc]`
//! macro generates the server dispatch, the Rust client, and the schema.

use serde::{de::DeserializeOwned, Serialize};
use std::sync::atomic::{AtomicU32, Ordering};

#[cfg(not(target_arch = "wasm32"))]
pub mod native;

#[cfg(target_arch = "wasm32")]
pub mod web {
    use crate::{RpcError, RpcResult};
    pub fn decode_frame(msg: &[u8]) -> RpcResult<(u32, &[u8])> {
        if msg.len() < 4 { return Err(RpcError::Transport("short frame".into())); }
        let method = u32::from_le_bytes([msg[0], msg[1], msg[2], msg[3]]);
        Ok((method, &msg[4..]))
    }
    pub fn frame_response(resp: &[u8]) -> Vec<u8> { resp.to_vec() }
}

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
/// Producers call [`RingBuffer::write`], consumers call [`RingBuffer::read`].
/// Both are lock-free (atomic Acquire/Release).
pub struct RingBuffer<'a> {
    capacity: u32,
    write_idx: &'a AtomicU32,
    read_idx: &'a AtomicU32,
    data: &'a [u8],
}

const HEADER_SIZE: usize = 12; // capacity(4) + write_idx(4) + read_idx(4)

impl<'a> RingBuffer<'a> {
    /// Initialize a ring buffer in a fresh backing buffer of `total_size` bytes.
    /// The first `HEADER_SIZE` bytes are the header; the rest is the data ring.
    pub fn init(buf: &mut [u8]) {
        let cap = (buf.len() - HEADER_SIZE) as u32;
        buf[..4].copy_from_slice(&cap.to_le_bytes());
        AtomicU32::from(0u32).store(0, Ordering::Relaxed); // won't persist — see below
        // Write header directly:
        let w = 0u32.to_le_bytes();
        let r = 0u32.to_le_bytes();
        buf[4..8].copy_from_slice(&w);
        buf[8..12].copy_from_slice(&r);
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

        // Write frame: [len][payload], wrapping around.
        let data_len = self.capacity as usize;
        let offset = (w as usize) % data_len;
        let len_bytes = (payload.len() as u32).to_le_bytes();

        // Write length prefix
        self.write_bytes(offset, &len_bytes);
        // Write payload
        let payload_offset = (offset + 4) % data_len;
        self.write_bytes(payload_offset, payload);

        // Publish (Release fence)
        self.write_idx.store(w.wrapping_add(frame_len), Ordering::Release);
        Ok(())
    }

    /// Read the next framed message from the ring buffer.
    /// Returns `Err(BufferEmpty)` if no message is available.
    pub fn read(&self) -> RpcResult<Vec<u8>> {
        let w = self.write_idx.load(Ordering::Acquire);
        let r = self.read_idx.load(Ordering::Relaxed);
        if w == r {
            return Err(RpcError::BufferEmpty);
        }

        let data_len = self.capacity as usize;
        let offset = (r as usize) % data_len;

        // Read length prefix
        let len_bytes = self.read_bytes(offset, 4);
        let payload_len = u32::from_le_bytes(len_bytes.try_into().unwrap()) as usize;
        let frame_len = 4 + payload_len as u32;

        // Read payload
        let payload_offset = (offset + 4) % data_len;
        let payload = self.read_bytes(payload_offset, payload_len);

        // Consume (Release fence)
        self.read_idx.store(r.wrapping_add(frame_len), Ordering::Release);
        Ok(payload)
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
            // SAFETY: both sides have access to the shared buffer. The writer
            // owns the region [w, w+len) (guaranteed by the capacity check).
            unsafe {
                let dst = self.data.as_ptr().add(offset) as *mut u8;
                std::ptr::copy_nonoverlapping(src.as_ptr(), dst, src.len());
            }
        } else {
            // Wrap around
            let first = data_len - offset;
            unsafe {
                let dst = self.data.as_ptr().add(offset) as *mut u8;
                std::ptr::copy_nonoverlapping(src.as_ptr(), dst, first);
                let dst2 = self.data.as_ptr() as *mut u8;
                std::ptr::copy_nonoverlapping(src.as_ptr().add(first), dst2, src.len() - first);
            }
        }
    }

    fn read_bytes(&self, offset: usize, len: usize) -> Vec<u8> {
        let data_len = self.capacity as usize;
        let mut out = vec![0u8; len];
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
        out
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
    fn call(&self, service: &str, method: u32, args: &[u8]) -> RpcResult<Vec<u8>> {
        // Frame: [method: u32][args...]
        let mut frame = Vec::with_capacity(4 + args.len());
        frame.extend_from_slice(&method.to_le_bytes());
        frame.extend_from_slice(args);
        self.request.write(&frame)?;
        // Block until response is available (poll — or use Atomics.wait on web)
        loop {
            match self.response.read() {
                Ok(resp) => return Ok(resp),
                Err(RpcError::BufferEmpty) => {
                    std::hint::spin_loop();
                }
                Err(e) => return Err(e),
            }
        }
    }
}

// --- in-memory loopback (for tests + same-process workers) ----------------

/// In-memory loopback: handy for tests and for worker<->worker calls within the
/// same process. Uses RingBuffer over a plain `Vec<u8>` backing.
pub struct LoopbackTransport {
    _request_buf: Vec<u8>,
    _response_buf: Vec<u8>,
    // The RingBuffers borrow the bufs — we need to keep them alive.
    // In practice, use `LoopbackTransport::new()` which returns owned handles.
}

impl LoopbackTransport {
    /// Create a pair of (caller, worker) transports backed by ring buffers.
    pub fn new_pair(capacity: usize) -> (LoopbackCaller, LoopbackWorker) {
        let mut req_buf = vec![0u8; capacity + 12];
        let mut resp_buf = vec![0u8; capacity + 12];
        RingBuffer::init(&mut req_buf);
        RingBuffer::init(&mut resp_buf);
        (
            LoopbackCaller { req_buf, resp_buf },
            LoopbackWorker { req_buf: Vec::new(), resp_buf: Vec::new() },
        )
    }
}

pub struct LoopbackCaller {
    req_buf: Vec<u8>,
    resp_buf: Vec<u8>,
}

pub struct LoopbackWorker {
    req_buf: Vec<u8>,
    resp_buf: Vec<u8>,
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
