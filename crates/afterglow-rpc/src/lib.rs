//! # afterglow-rpc
//!
//! Shared-memory SPSC ring buffers + a postcard codec for afterglow-engine
//! worker communication.
//!
//! Per worker, two ring buffers (one per direction):
//! - **Request** (caller → worker): the caller writes framed requests.
//! - **Response** (worker → caller): the worker writes framed responses.
//!
//! Each ring buffer is a lock-free SPSC circular buffer with atomic read/write
//! indices over a 4-byte-aligned shared region. The producer calls
//! [`RingBuffer::write`]; the consumer calls [`RingBuffer::read`] /
//! [`RingBuffer::read_into`]. Both are lock-free (Acquire/Release).
//!
//! - **Wire codec**: [`postcard`] (compact, serde-based).
//! - **Framing**: every ring message is `[len: u32 LE][payload]`.
//! - **Response envelope**: worker responses are wrapped in [`Response`] so a
//!   server error, a decode failure, or a unit/zero-byte result are all
//!   distinguishable from a successful payload.
//! - **Native**: [`native`] — OS threads + a compact heap-backed allocation.
//! - **Web**: `afterglow-web` — `SharedArrayBuffer`-backed wasm memory.
//!
//! Interfaces are defined once in Rust; the `afterglow-rpc-macros` `#[rpc]`
//! macro generates the server trait (with a provided `serve` dispatch), the
//! typed Rust client, and the schema.

use std::cell::Cell;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU32, Ordering};

#[cfg(not(target_arch = "wasm32"))]
pub mod native;

pub type RpcResult<T> = Result<T, RpcError>;

/// All errors produced by the RPC runtime: codec/framing, ring-buffer
/// state, transport, and worker lifecycle.
#[derive(Debug)]
pub enum RpcError {
    /// Postcard encode/decode failure on a user value.
    Codec(String),
    /// The requested method id is not part of the trait.
    UnknownMethod,
    /// Ring buffer is full (no room to write the frame).
    BufferFull,
    /// Ring buffer is empty (no frame to read).
    BufferEmpty,
    /// `read_into` buffer too small for the next frame. The frame is left in
    /// the ring so the caller can retry with a larger buffer.
    BufferTooSmall { needed: u32, provided: u32 },
    /// The ring buffer state is inconsistent (bad capacity, truncated/corrupt
    /// frame header, advertised length exceeding available bytes).
    CorruptFrame(String),
    /// A backing region failed validation (too small, misaligned, bad capacity).
    BadBacking(String),
    /// Low-level transport failure not covered by a more specific variant.
    Transport(String),
    /// The server returned an error (preserved verbatim).
    Server(String),
    /// The server could not decode the request arguments.
    Decode(String),
    /// The worker thread/loop is no longer running.
    WorkerDead,
    /// No response arrived before the bounded wait deadline.
    Timeout,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Codec(s) => write!(f, "rpc codec: {s}"),
            Self::UnknownMethod => write!(f, "rpc unknown method"),
            Self::BufferFull => write!(f, "rpc ring buffer full"),
            Self::BufferEmpty => write!(f, "rpc ring buffer empty"),
            Self::BufferTooSmall { needed, provided } => {
                write!(
                    f,
                    "rpc read buffer too small: needed {needed}, provided {provided}"
                )
            }
            Self::CorruptFrame(s) => write!(f, "rpc corrupt frame: {s}"),
            Self::BadBacking(s) => write!(f, "rpc bad backing: {s}"),
            Self::Transport(s) => write!(f, "rpc transport: {s}"),
            Self::Server(s) => write!(f, "rpc server error: {s}"),
            Self::Decode(s) => write!(f, "rpc decode error: {s}"),
            Self::WorkerDead => write!(f, "rpc worker dead"),
            Self::Timeout => write!(f, "rpc timeout"),
        }
    }
}
impl std::error::Error for RpcError {}

// --- codec helpers (used by generated code) -------------------------------

pub fn encode<T: serde::Serialize>(v: &T) -> RpcResult<Vec<u8>> {
    postcard::to_allocvec(v).map_err(|e| RpcError::Codec(e.to_string()))
}
pub fn decode<T: serde::de::DeserializeOwned>(b: &[u8]) -> RpcResult<T> {
    postcard::from_bytes(b).map_err(|e| RpcError::Codec(e.to_string()))
}

// --- transport trait ------------------------------------------------------

/// The byte-pipe a generated client talks over. A `call` writes a framed
/// request `[method: u32 LE][args]` and returns the response payload bytes
/// (already unwrapped from the [`Response`] envelope) or an [`RpcError`].
pub trait Transport {
    fn call(&self, service: &str, method: u32, args: &[u8]) -> RpcResult<Vec<u8>>;
}

// --- response envelope ----------------------------------------------------

/// Wire envelope for worker responses, postcard-encoded on the ring.
///
/// Every response — success, server error, decode failure, or a unit/empty
/// return value — is wrapped in this enum so the client can always tell them
/// apart. The transport decodes it and either returns the `Ok` payload or maps
/// the error to an [`RpcError`] (preserving the server's meaning).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum Response {
    /// Successful call; `payload` is the postcard-encoded return value (may be
    /// empty for unit/zero-byte results).
    Ok(Vec<u8>),
    /// `serve` returned an error. `method` is the request method id.
    Server { method: u32, message: String },
    /// The server could not decode the request arguments. `method` is the
    /// request method id.
    Decode { method: u32, message: String },
}

/// Wrap a `serve` result into a [`Response`] envelope, preserving the inner
/// message of `Server` / `Decode` errors verbatim.
pub fn make_response(method: u32, res: RpcResult<Vec<u8>>) -> Response {
    match res {
        Ok(payload) => Response::Ok(payload),
        Err(RpcError::Codec(msg)) => Response::Decode {
            method,
            message: msg,
        },
        Err(RpcError::Decode(msg)) => Response::Decode {
            method,
            message: msg,
        },
        Err(RpcError::Server(msg)) => Response::Server {
            method,
            message: msg,
        },
        Err(RpcError::UnknownMethod) => Response::Server {
            method,
            message: "unknown method".into(),
        },
        Err(e) => Response::Server {
            method,
            message: e.to_string(),
        },
    }
}

/// Decode a [`Response`] envelope and extract the `Ok` payload, or map a
/// server/decode error into the corresponding [`RpcError`].
pub fn unwrap_response(bytes: &[u8]) -> RpcResult<Vec<u8>> {
    let resp: Response = decode(bytes)?;
    match resp {
        Response::Ok(payload) => Ok(payload),
        Response::Server { message, .. } => Err(RpcError::Server(message)),
        Response::Decode { message, .. } => Err(RpcError::Decode(message)),
    }
}

// --- ring buffer ----------------------------------------------------------

/// 4-byte-aligned header of a ring buffer, stored at the start of every
/// backing region. Fields are private: a backing region is constructed via
/// [`RingHeader::new`] (and written in place with `ptr::write`), then shared.
/// The web target later constructs `RingHeader + UnsafeCell<[u8; N]>` and views
/// it through [`RingBuffer::from_header_data`].
///
/// `repr(C, align(4))`: `capacity` at offset 0, `write_idx` at 4, `read_idx`
/// at 8 — exactly [`HEADER_SIZE`] bytes.
#[repr(C, align(4))]
#[derive(Debug)]
pub struct RingHeader {
    capacity: u32,
    write_idx: AtomicU32,
    read_idx: AtomicU32,
}

impl RingHeader {
    /// Construct a fresh header for a ring of `capacity` data bytes with zeroed
    /// indices (empty ring). `const`, so usable in `static` initializers.
    pub const fn new(capacity: u32) -> Self {
        Self {
            capacity,
            write_idx: AtomicU32::new(0),
            read_idx: AtomicU32::new(0),
        }
    }
    /// The data-area capacity (excluding the 12-byte header).
    pub fn capacity(&self) -> u32 {
        self.capacity
    }
    /// Reset both indices to zero (empty ring). Use only with exclusive access
    /// (e.g. at startup), before the region is shared across threads.
    pub fn reset(&self) {
        self.write_idx.store(0, Ordering::Release);
        self.read_idx.store(0, Ordering::Release);
    }
}

/// Header size in bytes: `capacity + write_idx + read_idx` (3 × u32).
pub const HEADER_SIZE: usize = 12;
/// Required alignment of a ring buffer backing region (AtomicU32 alignment).
pub const ALIGN: usize = 4;

/// A borrowed view over a 4-byte-aligned, initialized ring buffer region.
///
/// Constructed via [`RingBuffer::from_header_data`] from an *actually
/// constructed* [`RingHeader`] plus a raw data pointer/len (never by casting
/// arbitrary bytes to a header). The view is `!Send + !Sync`; cross-thread use
/// goes through the split native halves ([`native::RingProducer`] /
/// [`native::RingConsumer`]).
///
/// # Soundness
///
/// The data area must be `UnsafeCell`-backed (interior-mutable) and in a
/// `Sync` location when shared across threads. Mutation happens only via raw
/// `*mut u8` pointers derived from the data area (never forming
/// `&[u8]`/`&mut [u8]` over the whole array, which would alias between the
/// SPSC halves). The producer writes a frame and only then publishes
/// `write_idx` (Release); the consumer reads those bytes only after observing
/// `write_idx` (Acquire) and only then publishes `read_idx` (Release). No two
/// threads mutate the same byte, and each byte is published through an atomic
/// release/acquire pair.
pub struct RingBuffer<'a> {
    header: &'a RingHeader,
    data: std::ptr::NonNull<u8>,
    capacity: u32,
    _not_sync: PhantomData<Cell<()>>,
}

impl<'a> RingBuffer<'a> {
    /// Wrap an actually-constructed [`RingHeader`] plus a raw data area.
    ///
    /// Used by both the web target (`RingHeader + UnsafeCell<[u8; N]>` in a
    /// `Sync` `static`) and the native owned storage ([`native::RingStorage`],
    /// which writes the header in place with `ptr::write`).
    ///
    /// # Safety
    /// - `header` is a valid, constructed `RingHeader` (via [`RingHeader::new`]
    ///   or `ptr::write` of one), whose `capacity` equals `data_len`, and
    ///   remains valid for `'a`;
    /// - `data` is a non-null, valid pointer to `data_len` writable bytes that
    ///   remain valid for `'a`;
    /// - the data area is `UnsafeCell`-backed (interior-mutable) and in a
    ///   `Sync` location if shared across threads;
    /// - the caller upholds the SPSC contract (one producer, one consumer).
    pub unsafe fn from_header_data(
        header: &'a RingHeader,
        data: *mut u8,
        data_len: usize,
    ) -> RpcResult<Self> {
        if data.is_null() {
            return Err(RpcError::BadBacking("null data pointer".into()));
        }
        if data_len == 0 || data_len > u32::MAX as usize {
            return Err(RpcError::BadBacking(format!(
                "invalid data length {data_len}"
            )));
        }
        if header.capacity() as usize != data_len {
            return Err(RpcError::BadBacking(format!(
                "header capacity {} != data length {data_len}",
                header.capacity()
            )));
        }
        // SAFETY: checked non-null above.
        let data = unsafe { std::ptr::NonNull::new_unchecked(data) };
        Ok(Self {
            header,
            data,
            capacity: header.capacity(),
            _not_sync: PhantomData,
        })
    }

    /// Data-area capacity (excluding the 12-byte header).
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Non-blocking: is there at least one framed message available?
    pub fn has_data(&self) -> bool {
        self.header.write_idx.load(Ordering::Acquire)
            != self.header.read_idx.load(Ordering::Relaxed)
    }

    /// Payload length of the next frame, without consuming it.
    ///
    /// Returns `Err(BufferEmpty)` if empty, or `Err(CorruptFrame)` if the
    /// header is truncated, the indices are inconsistent (`used > capacity`),
    /// or the advertised length exceeds available bytes.
    pub fn peek_len(&self) -> RpcResult<u32> {
        let w = self.header.write_idx.load(Ordering::Acquire);
        let r = self.header.read_idx.load(Ordering::Relaxed);
        let used = w.wrapping_sub(r);
        let cap = self.capacity;
        if used == 0 {
            return Err(RpcError::BufferEmpty);
        }
        if used > cap {
            return Err(RpcError::CorruptFrame(format!(
                "used {used} > capacity {cap}"
            )));
        }
        if used < 4 {
            return Err(RpcError::CorruptFrame(format!(
                "truncated frame header: {used} bytes available"
            )));
        }
        let off = (r as usize) % cap as usize;
        let mut len_bytes = [0u8; 4];
        // SAFETY: SPSC consumer; `r` was published by the producer (Release on
        // write_idx), so the 4 header bytes are visible and stable until we
        // advance read_idx. `off..off+4` is within `[0, cap)` (validated).
        unsafe { self.copy_out(off, &mut len_bytes) };
        let payload_len = u32::from_le_bytes(len_bytes);
        let frame_len = 4u32
            .checked_add(payload_len)
            .ok_or_else(|| RpcError::CorruptFrame("frame length overflow".into()))?;
        if frame_len > used {
            return Err(RpcError::CorruptFrame(format!(
                "advertised frame {frame_len} > available {used}"
            )));
        }
        if payload_len as usize > cap as usize {
            return Err(RpcError::CorruptFrame(format!(
                "payload {payload_len} > capacity {cap}"
            )));
        }
        Ok(payload_len)
    }

    /// Write a framed message `[len: u32][payload]`. Returns `Err(BufferFull)`
    /// if the frame cannot fit (now or ever, if larger than capacity).
    pub fn write(&self, payload: &[u8]) -> RpcResult<()> {
        let cap = self.capacity;
        if payload.len() > (u32::MAX as usize).saturating_sub(4) {
            return Err(RpcError::CorruptFrame(
                "payload length overflows u32".into(),
            ));
        }
        let frame_len = 4 + payload.len() as u32;
        if frame_len > cap {
            // Can never fit, even in an empty buffer.
            return Err(RpcError::BufferFull);
        }
        let w = self.header.write_idx.load(Ordering::Relaxed);
        let r = self.header.read_idx.load(Ordering::Acquire);
        let used = w.wrapping_sub(r);
        if used > cap {
            return Err(RpcError::CorruptFrame(format!(
                "used {used} > capacity {cap}"
            )));
        }
        let free = cap - used;
        if frame_len > free {
            return Err(RpcError::BufferFull);
        }
        let cap_us = cap as usize;
        let off = (w as usize) % cap_us;
        let len_bytes = (payload.len() as u32).to_le_bytes();
        // SAFETY: SPSC producer; the bytes `[w..w+frame_len]` are not yet
        // visible (write_idx not published) and cannot alias the consumer,
        // which only touches bytes it has already consumed (`< read_idx`).
        unsafe {
            self.copy_in(off, &len_bytes);
            self.copy_in((off + 4) % cap_us, payload);
        }
        self.header
            .write_idx
            .store(w.wrapping_add(frame_len), Ordering::Release);
        Ok(())
    }

    /// Read the next framed message, allocating a `Vec`. Prefer
    /// [`read_into`](Self::read_into) for allocation-free reads.
    pub fn read(&self) -> RpcResult<Vec<u8>> {
        let payload_len = self.peek_len()?;
        let mut out = vec![0u8; payload_len as usize];
        let r = self.header.read_idx.load(Ordering::Relaxed);
        let cap_us = self.capacity as usize;
        let off = (r as usize) % cap_us;
        // SAFETY: `peek_len` validated the frame; consumer reads released bytes.
        unsafe { self.copy_out((off + 4) % cap_us, &mut out) };
        self.header
            .read_idx
            .store(r.wrapping_add(4 + payload_len), Ordering::Release);
        Ok(out)
    }

    /// Read the next frame directly into `out`. If `out` is too small, returns
    /// `Err(BufferTooSmall { needed, provided })` and **leaves the frame in the
    /// ring** so the caller can retry with a larger buffer. Never truncates.
    pub fn read_into(&self, out: &mut [u8]) -> RpcResult<usize> {
        let payload_len = self.peek_len()?;
        if (payload_len as usize) > out.len() {
            return Err(RpcError::BufferTooSmall {
                needed: payload_len,
                provided: out.len() as u32,
            });
        }
        let r = self.header.read_idx.load(Ordering::Relaxed);
        let cap_us = self.capacity as usize;
        let off = (r as usize) % cap_us;
        // SAFETY: `peek_len` validated the frame; copy the payload.
        unsafe { self.copy_out((off + 4) % cap_us, &mut out[..payload_len as usize]) };
        self.header
            .read_idx
            .store(r.wrapping_add(4 + payload_len), Ordering::Release);
        Ok(payload_len as usize)
    }

    // --- internal raw copy helpers ----------------------------------------
    //
    // Operate on raw pointers derived from the `UnsafeCell` data area, never
    // forming `&[u8]`/`&mut [u8]` over the whole array (which would alias
    // between the SPSC halves). Wraparound is handled with a two-segment copy.

    /// # Safety
    /// Caller (the producer) must ensure `[off..off+src.len())` (modulo
    /// capacity) is its exclusive region under the SPSC contract.
    #[inline]
    unsafe fn copy_in(&self, off: usize, src: &[u8]) {
        let cap = self.capacity as usize;
        let base = self.data.as_ptr();
        if off + src.len() <= cap {
            // SAFETY: in-bounds, disjoint from `src` (caller-guaranteed SPSC).
            unsafe {
                std::ptr::copy_nonoverlapping(src.as_ptr(), base.add(off), src.len());
            }
        } else {
            let first = cap - off;
            // SAFETY: two disjoint segments, both in `[0, cap)`.
            unsafe {
                std::ptr::copy_nonoverlapping(src.as_ptr(), base.add(off), first);
                std::ptr::copy_nonoverlapping(src.as_ptr().add(first), base, src.len() - first);
            }
        }
    }

    /// # Safety
    /// Caller (the consumer) must ensure `[off..off+out.len())` (modulo
    /// capacity) is a frame the producer has published (write_idx released).
    #[inline]
    unsafe fn copy_out(&self, off: usize, out: &mut [u8]) {
        let cap = self.capacity as usize;
        let base = self.data.as_ptr();
        if off + out.len() <= cap {
            // SAFETY: in-bounds, disjoint from `out` (caller-guaranteed SPSC).
            unsafe {
                std::ptr::copy_nonoverlapping(base.add(off), out.as_mut_ptr(), out.len());
            }
        } else {
            let first = cap - off;
            // SAFETY: two disjoint segments, both in `[0, cap)`.
            unsafe {
                std::ptr::copy_nonoverlapping(base.add(off), out.as_mut_ptr(), first);
                std::ptr::copy_nonoverlapping(base, out.as_mut_ptr().add(first), out.len() - first);
            }
        }
    }
}

// --- schema (for the `dump-schema` bin; no TS codegen exists) --------------

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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// 4-byte-aligned stack buffer for ring buffer tests. `ring()` constructs
    /// a real `RingHeader` in place (via `ptr::write`) — never a byte cast.
    #[repr(C, align(4))]
    struct AlignedBuf<const N: usize>([u8; N]);

    impl<const N: usize> AlignedBuf<N> {
        fn new() -> Self {
            Self([0u8; N])
        }
        /// Construct a fresh `RingHeader` at offset 0 (data capacity =
        /// `N - HEADER_SIZE`) and return a `RingBuffer` view. Data is zeroed.
        fn ring(&mut self) -> RingBuffer<'_> {
            let cap = (N - HEADER_SIZE) as u32;
            // SAFETY: `self.0` is 4-aligned (`repr(align(4))`) and `N` is large
            // enough at every call site (≥ HEADER_SIZE+1). Writing a
            // `RingHeader` (repr(C, align(4)), HEADER_SIZE bytes) at offset 0
            // is in-bounds and aligned.
            unsafe {
                std::ptr::write(self.0.as_mut_ptr() as *mut RingHeader, RingHeader::new(cap));
            }
            unsafe { self.view() }
        }
        /// Write a little-endian `u32` at byte offset `off` (for corrupting the
        /// header indices before constructing a view).
        fn set_u32(&mut self, off: usize, val: u32) {
            self.0[off..off + 4].copy_from_slice(&val.to_le_bytes());
        }
        /// View over a region whose header has already been constructed.
        ///
        /// # Safety
        /// The caller must have written a valid `RingHeader` at offset 0.
        unsafe fn view(&self) -> RingBuffer<'_> {
            let base = self.0.as_ptr();
            // SAFETY: caller wrote a `RingHeader` at offset 0; `base` is
            // 4-aligned; the data area `[HEADER_SIZE..N]` follows.
            unsafe {
                let header = &*(base as *const RingHeader);
                let data = base.add(HEADER_SIZE) as *mut u8;
                RingBuffer::from_header_data(header, data, header.capacity() as usize)
                    .expect("AlignedBuf invariant: constructed ring buffer")
            }
        }
    }

    #[test]
    fn empty_full_exact() {
        let mut buf = AlignedBuf::<76>::new(); // header 12 + data 64
        let rb = buf.ring();
        assert!(matches!(rb.read(), Err(RpcError::BufferEmpty)));
        assert!(!rb.has_data());
        assert!(matches!(rb.peek_len(), Err(RpcError::BufferEmpty)));

        // exact-fit: capacity 64, frame = 4 + 60 = 64.
        let p = vec![1u8; 60];
        rb.write(&p).unwrap();
        assert!(rb.has_data());
        assert_eq!(rb.peek_len().unwrap(), 60);
        let out = rb.read().unwrap();
        assert_eq!(out, p);
        assert!(!rb.has_data());

        // one byte over capacity -> can never fit.
        assert!(matches!(rb.write(&[0u8; 61]), Err(RpcError::BufferFull)));
    }

    #[test]
    fn oversized_write_to_full() {
        let mut buf = AlignedBuf::<44>::new(); // header 12 + data 32
        let rb = buf.ring();
        // frame 4+20=24 fits; writing again (48 > 32 free after first = 8) is full.
        rb.write(&[9u8; 20]).unwrap();
        assert!(matches!(rb.write(&[9u8; 20]), Err(RpcError::BufferFull)));
        // draining makes room.
        rb.read().unwrap();
        rb.write(&[9u8; 20]).unwrap();
    }

    #[test]
    fn read_into_too_small_leaves_frame() {
        let mut buf = AlignedBuf::<140>::new(); // header 12 + data 128
        let rb = buf.ring();
        rb.write(&[7u8; 40]).unwrap();
        let mut small = [0u8; 10];
        match rb.read_into(&mut small) {
            Err(RpcError::BufferTooSmall { needed, provided }) => {
                assert_eq!(needed, 40);
                assert_eq!(provided, 10);
            }
            other => panic!("expected BufferTooSmall, got {other:?}"),
        }
        // frame still available — not consumed.
        assert!(rb.has_data());
        assert_eq!(rb.peek_len().unwrap(), 40);
        let mut big = vec![0u8; 64];
        let n = rb.read_into(&mut big).unwrap();
        assert_eq!(n, 40);
        assert!(!rb.has_data());
    }

    #[test]
    fn wraparound_including_header_wrap() {
        // Capacity 16. Frame A = 4+10 = 14 (write_idx 0..14). Read it so the
        // next write starts at off 14 -> frame B's 4-byte header straddles
        // the end (bytes 14,15,0,1), i.e. a header wrap.
        let mut buf = AlignedBuf::<28>::new(); // header 12 + data 16
        let rb = buf.ring();
        rb.write(&[0xAA; 10]).unwrap(); // frame A, w=14
        assert_eq!(rb.read().unwrap(), vec![0xAA; 10]); // r=14
        rb.write(&[0xBB; 2]).unwrap(); // frame B header wraps at off 14
        assert_eq!(rb.read().unwrap(), vec![0xBB; 2]);
        assert!(!rb.has_data());

        // Payload wrap: frame B's payload (not header) straddles the end.
        let mut buf = AlignedBuf::<28>::new();
        let rb = buf.ring();
        rb.write(&[1u8; 2]).unwrap(); // frame 4+2=6, w=6
        assert_eq!(rb.read().unwrap(), vec![1u8; 2]); // r=6, free=16
        rb.write(&[2u8; 10]).unwrap(); // frame 4+10=14, header at 6, payload at 10 wraps
        assert_eq!(rb.read().unwrap(), vec![2u8; 10]);
    }

    #[test]
    fn many_messages_wrap_stress() {
        let mut buf = AlignedBuf::<268>::new(); // header 12 + data 256
        let rb = buf.ring();
        let mut seq = 0u32;
        for _ in 0..1000 {
            let payload = seq.to_le_bytes().repeat(7); // 28 bytes, frame 32
            rb.write(&payload).unwrap();
            let out = rb.read().unwrap();
            assert_eq!(out, payload);
            seq = seq.wrapping_add(1);
        }
    }

    #[test]
    fn truncated_frame_header_rejected() {
        // Corrupt indices: write_idx=2, read_idx=0 -> used=2 < 4.
        let mut buf = AlignedBuf::<76>::new();
        buf.ring(); // construct the header
        buf.set_u32(4, 2); // write_idx = 2
        // SAFETY: header constructed above.
        let rb = unsafe { buf.view() };
        match rb.peek_len() {
            Err(RpcError::CorruptFrame(_)) => {}
            other => panic!("expected CorruptFrame, got {other:?}"),
        }
    }

    #[test]
    fn used_exceeds_capacity_rejected() {
        // Corrupt indices: write_idx = cap+10 -> used > capacity.
        let mut buf = AlignedBuf::<76>::new(); // cap 64
        buf.ring();
        buf.set_u32(4, 74); // write_idx = 64 + 10 = 74
        // SAFETY: header constructed above.
        let rb = unsafe { buf.view() };
        assert!(matches!(rb.peek_len(), Err(RpcError::CorruptFrame(_))));
        assert!(matches!(
            rb.write(&[0u8; 1]),
            Err(RpcError::CorruptFrame(_))
        ));
    }

    #[test]
    fn advertised_len_exceeds_available_rejected() {
        let mut buf = AlignedBuf::<76>::new();
        let rb = buf.ring();
        rb.write(&[0u8; 4]).unwrap(); // frame 8 bytes, used=8
        // Overwrite the length field at data offset 0 to 100 (> used 8).
        // SAFETY: producer-side raw write of 4 bytes within `[0, cap)`.
        unsafe { rb.copy_in(0, &100u32.to_le_bytes()) };
        match rb.peek_len() {
            Err(RpcError::CorruptFrame(_)) => {}
            other => panic!("expected CorruptFrame, got {other:?}"),
        }
    }

    #[test]
    fn from_header_data_validation() {
        let hdr = RingHeader::new(16);
        // null data
        assert!(unsafe { RingBuffer::from_header_data(&hdr, std::ptr::null_mut(), 16) }.is_err());
        // zero data length
        let mut data = [0u8; 16];
        assert!(unsafe { RingBuffer::from_header_data(&hdr, data.as_mut_ptr(), 0) }.is_err());
        // capacity mismatch: header says 16, data length 8
        let mut small = [0u8; 8];
        assert!(unsafe { RingBuffer::from_header_data(&hdr, small.as_mut_ptr(), 8) }.is_err());
        // valid
        let mut data = [0u8; 16];
        let rb = unsafe { RingBuffer::from_header_data(&hdr, data.as_mut_ptr(), 16) }.unwrap();
        assert_eq!(rb.capacity(), 16);
    }

    #[test]
    fn header_new_reset_capacity() {
        let h = RingHeader::new(123);
        assert_eq!(h.capacity(), 123);
        assert_eq!(h.write_idx.load(Ordering::Relaxed), 0);
        assert_eq!(h.read_idx.load(Ordering::Relaxed), 0);
        h.write_idx.store(7, Ordering::Relaxed);
        h.reset();
        assert_eq!(h.write_idx.load(Ordering::Relaxed), 0);
        assert_eq!(h.read_idx.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn envelope_roundtrip() {
        let r = make_response(3, Ok(vec![1, 2, 3]));
        let bytes = encode(&r).unwrap();
        let p = unwrap_response(&bytes).unwrap();
        assert_eq!(p, vec![1, 2, 3]);

        let r = make_response(3, Err(RpcError::UnknownMethod));
        let bytes = encode(&r).unwrap();
        match unwrap_response(&bytes) {
            Err(RpcError::Server(m)) => assert_eq!(m, "unknown method"),
            other => panic!("expected Server, got {other:?}"),
        }

        let r = make_response(3, Err(RpcError::Codec("bad args".into())));
        let bytes = encode(&r).unwrap();
        match unwrap_response(&bytes) {
            Err(RpcError::Decode(m)) => assert_eq!(m, "bad args"),
            other => panic!("expected Decode, got {other:?}"),
        }

        // unit / zero-byte ok result is distinguishable.
        let r = make_response(0, Ok(Vec::new()));
        let bytes = encode(&r).unwrap();
        assert_eq!(unwrap_response(&bytes).unwrap(), Vec::<u8>::new());
    }
}
