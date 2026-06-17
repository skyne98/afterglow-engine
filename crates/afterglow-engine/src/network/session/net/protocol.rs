use std::io;

use crate::network::session::{SessionEvent, SessionRequest};

/// Wire format: every message is a 4-byte little-endian length followed by
/// postcard-encoded payload bytes.
/// Maximum allowed packet size on the wire. Bounds memory growth from
/// adversarial length prefixes (would-be OOM).
pub(crate) const MAX_PACKET_SIZE: usize = 64 * 1024;

pub(crate) fn try_pop_request(buf: &mut Vec<u8>) -> io::Result<Option<SessionRequest>> {
    pop_frame(buf, |bytes| {
        postcard::from_bytes(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    })
}

pub(crate) fn try_pop_event(buf: &mut Vec<u8>) -> io::Result<Option<SessionEvent>> {
    pop_frame(buf, |bytes| {
        postcard::from_bytes(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    })
}

/// Pops one length-prefixed frame from `buf` and hands its payload to
/// `decode`. Returns `Ok(None)` if fewer than `4 + len` bytes are available.
/// Returns `Err(InvalidData)` if the announced length exceeds
/// [`MAX_PACKET_SIZE`].
fn pop_frame<T>(
    buf: &mut Vec<u8>,
    decode: impl FnOnce(&[u8]) -> io::Result<T>,
) -> io::Result<Option<T>> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if len > MAX_PACKET_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame length {len} exceeds MAX_PACKET_SIZE"),
        ));
    }
    if buf.len() < 4 + len {
        return Ok(None);
    }
    // Drain the length prefix AND the payload so the next frame starts at
    // the next length prefix.
    let payload: Vec<u8> = buf.drain(..4 + len).skip(4).collect();
    decode(&payload).map(Some)
}

/// Serialize a request to bytes ready to be pushed onto a connection's write
/// buffer (length-prefixed postcard payload).
pub(crate) fn encode_request(request: &SessionRequest) -> io::Result<Vec<u8>> {
    encode_frame(
        postcard::to_allocvec(request)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
    )
}

pub(crate) fn encode_event(event: &SessionEvent) -> io::Result<Vec<u8>> {
    encode_frame(
        postcard::to_allocvec(event).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
    )
}

fn encode_frame(payload: Vec<u8>) -> io::Result<Vec<u8>> {
    let len = payload.len() as u32;
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

pub(crate) fn write_event(writer: &mut dyn io::Write, event: &SessionEvent) -> io::Result<()> {
    let bytes = encode_event(event)?;
    writer.write_all(&bytes)?;
    writer.flush()
}
