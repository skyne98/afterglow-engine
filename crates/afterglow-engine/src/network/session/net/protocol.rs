use std::io;

use crate::network::session::{SessionEvent, SessionRequest};

/// Wire format: every message is a 4-byte little-endian length followed by
/// postcard-encoded payload bytes.
pub(crate) fn try_pop_request(buf: &mut Vec<u8>) -> io::Result<Option<SessionRequest>> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if buf.len() < 4 + len {
        return Ok(None);
    }
    let payload: Vec<u8> = buf.drain(..4 + len).skip(4).collect();
    postcard::from_bytes(&payload)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub(crate) fn try_pop_event(buf: &mut Vec<u8>) -> io::Result<Option<SessionEvent>> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if buf.len() < 4 + len {
        return Ok(None);
    }
    let payload: Vec<u8> = buf.drain(..4 + len).skip(4).collect();
    postcard::from_bytes(&payload)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub(crate) fn write_request(
    writer: &mut dyn io::Write,
    request: &SessionRequest,
) -> io::Result<()> {
    let bytes = postcard::to_allocvec(request)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    write_bytes(writer, &bytes)
}

pub(crate) fn write_event(
    writer: &mut dyn io::Write,
    event: &SessionEvent,
) -> io::Result<()> {
    let bytes = postcard::to_allocvec(event)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    write_bytes(writer, &bytes)
}

fn write_bytes(writer: &mut dyn io::Write, bytes: &[u8]) -> io::Result<()> {
    let len = bytes.len() as u32;
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(bytes)?;
    writer.flush()
}
