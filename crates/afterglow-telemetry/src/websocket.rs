//! Bounded WebSocket I/O for the profiling CLI and application I/O workers.
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket, protocol::WebSocketConfig};

pub const MAX_MESSAGE_BYTES: usize = 65_536;
pub type Socket = WebSocket<DeadlineStream>;

/// One absolute deadline includes all reads and writes for an operation.
#[derive(Debug)]
pub struct DeadlineStream {
    pub stream: TcpStream,
    deadline: Instant,
}
impl DeadlineStream {
    pub fn new(stream: TcpStream, timeout: Duration) -> io::Result<Self> {
        stream.set_nodelay(true)?;
        let mut result = Self { stream, deadline: Instant::now() };
        result.reset(timeout)?;
        Ok(result)
    }
    pub fn reset(&mut self, timeout: Duration) -> io::Result<()> {
        self.deadline = Instant::now().checked_add(timeout)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid WebSocket deadline"))?;
        Ok(())
    }
    fn remaining(&self) -> io::Result<Duration> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "Profiling deadline reached"));
        }
        Ok(remaining)
    }
}
impl Read for DeadlineStream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.stream.set_read_timeout(Some(self.remaining()?))?;
        self.stream.read(bytes)
    }
}
impl Write for DeadlineStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.stream.set_write_timeout(Some(self.remaining()?))?;
        self.stream.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

// The tungstenite callback must return its unboxed HTTP error response.
#[allow(clippy::result_large_err)]
pub fn accept(stream: TcpStream) -> io::Result<Socket> {
    let stream = DeadlineStream::new(stream, Duration::from_secs(5))?;
    tungstenite::accept_hdr_with_config(stream, |request: &tungstenite::handshake::server::Request, response| {
        // Browser origins are input boundaries, not a permissions system.
        let allowed = request.headers().get("origin").is_none_or(|origin| {
            origin.to_str().ok().and_then(|value| value.parse::<tungstenite::http::Uri>().ok())
                .is_some_and(|uri| matches!(uri.scheme_str(), Some("http" | "https"))
                    && matches!(uri.host(), Some("localhost" | "127.0.0.1" | "[::1]")))
        });
        if allowed && request.uri().path() == "/" { Ok(response) }
        else { Err(tungstenite::http::Response::builder().status(403).body(Some("Invalid profiling origin or path".into())).unwrap()) }
    }, Some(config())).map_err(io::Error::other)
}

pub fn config() -> WebSocketConfig {
    WebSocketConfig {
        write_buffer_size: 0,
        max_write_buffer_size: MAX_MESSAGE_BYTES + 1024,
        max_message_size: Some(MAX_MESSAGE_BYTES),
        max_frame_size: Some(MAX_MESSAGE_BYTES),
        ..Default::default()
    }
}

pub fn error(error: tungstenite::Error) -> io::Error {
    match error {
        tungstenite::Error::Io(error) => error,
        tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed =>
            io::Error::new(io::ErrorKind::UnexpectedEof, "Profiling peer disconnected"),
        tungstenite::Error::Protocol(tungstenite::error::ProtocolError::ResetWithoutClosingHandshake) =>
            io::Error::new(io::ErrorKind::UnexpectedEof, "Profiling peer disconnected"),
        other => io::Error::new(io::ErrorKind::InvalidData, other),
    }
}

pub fn send(socket: &mut Socket, opcode: u8, payload: &[u8], max_bytes: usize) -> io::Result<()> {
    if payload.len() >= max_bytes || max_bytes > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Profiling message exceeds capacity"));
    }
    let mut bytes = Vec::with_capacity(1 + payload.len());
    bytes.push(opcode);
    bytes.extend_from_slice(payload);
    socket.send(Message::Binary(bytes)).map_err(error)
}

/// Control frames do not reset the absolute capture deadline.
pub fn receive(socket: &mut Socket) -> io::Result<Option<Vec<u8>>> {
    loop {
        match socket.read().map_err(error)? {
            Message::Binary(bytes) if !bytes.is_empty() => return Ok(Some(bytes)),
            Message::Close(_) => return Ok(None),
            Message::Ping(_) | Message::Pong(_) => (),
            _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "Expected binary profiling message")),
        }
    }
}
