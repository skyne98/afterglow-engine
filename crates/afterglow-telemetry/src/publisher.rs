//! Application WebSocket client. The profiling CLI owns the server.
//! Call only on an I/O worker, never on a presentation or device callback thread.
use crate::connection::{self, CaptureHello};
use crate::websocket::{self, DeadlineStream, Socket};
use std::io;
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};
use tungstenite::Message;

pub struct Publisher {
    address: SocketAddr,
    socket: Option<Socket>,
    session: [u32; 4],
    epoch: u32,
    max_frame: usize,
    timeout: Duration,
    retry_at: Instant,
}
impl Publisher {
    /// Creation does not connect or start a thread.
    pub fn new(address: SocketAddr, session: [u32; 4], max_frame: usize, timeout: Duration) -> io::Result<Self> {
        if !address.ip().is_loopback() || address.port() == 0 || session == [0; 4]
            || !(256..=websocket::MAX_MESSAGE_BYTES).contains(&max_frame) || timeout.is_zero() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid profiling connection configuration"));
        }
        Ok(Self { address, socket: None, session, epoch: 0, max_frame, timeout, retry_at: Instant::now() })
    }
    pub fn address(&self) -> SocketAddr { self.address }
    pub fn epoch(&self) -> u32 { self.epoch }
    pub fn session(&self) -> [u32; 4] { self.session }
    pub fn connected(&self) -> bool { self.socket.is_some() }

    /// Retry at most once per second when the profiling server is unavailable.
    pub fn poll(&mut self) -> io::Result<bool> {
        if let Some(socket) = self.socket.as_mut() {
            socket.get_mut().reset(self.timeout)?;
            socket.get_mut().stream.set_nonblocking(true)?;
            let result = socket.read();
            let restore = socket.get_mut().stream.set_nonblocking(false);
            restore?;
            match result {
                Err(tungstenite::Error::Io(error)) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => return Ok(true),
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) if socket.flush().is_ok() => return Ok(true),
                _ => (),
            }
            self.socket.take();
            self.retry_at = Instant::now() + Duration::from_secs(1);
            return Ok(false);
        }
        if Instant::now() < self.retry_at { return Ok(false); }
        self.retry_at = Instant::now() + Duration::from_secs(1);
        let stream = match TcpStream::connect_timeout(&self.address, self.timeout) {
            Ok(stream) => stream,
            Err(_) => return Ok(false),
        };
        let stream = DeadlineStream::new(stream, self.timeout)?;
        let url = format!("ws://{}/", self.address);
        let (mut socket, _) = match tungstenite::client::client_with_config(url.as_str(), stream, Some(websocket::config())) {
            Ok(result) => result,
            Err(_) => return Ok(false),
        };
        let epoch = self.epoch.checked_add(1).ok_or_else(|| io::Error::other("Profiling epoch exhausted"))?;
        let hello = serde_json::to_vec(&CaptureHello {
            protocol: connection::PROTOCOL_VERSION, session: self.session, epoch, max_frame_bytes: self.max_frame,
        }).map_err(io::Error::other)?;
        websocket::send(&mut socket, connection::HELLO, &hello, self.max_frame)?;
        self.epoch = epoch;
        self.socket = Some(socket);
        Ok(true)
    }
    pub fn publish(&mut self, opcode: u8, payload: &[u8]) -> io::Result<bool> {
        if !matches!(opcode, connection::REGISTER | connection::BATCH | connection::METRICS) || payload.len() >= self.max_frame {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid profiling message"));
        }
        let Some(socket) = self.socket.as_mut() else { return Ok(false); };
        socket.get_mut().reset(self.timeout)?;
        if let Err(error) = websocket::send(socket, opcode, payload, self.max_frame) {
            self.socket.take();
            self.retry_at = Instant::now() + Duration::from_secs(1);
            return Err(error);
        }
        Ok(true)
    }
    pub fn finish(&mut self) -> io::Result<()> {
        self.retry_at = Instant::now() + Duration::from_secs(1);
        if let Some(mut socket) = self.socket.take() {
            socket.get_mut().reset(self.timeout)?;
            websocket::send(&mut socket, connection::FINISH, &[], self.max_frame)?;
            socket.close(None).map_err(websocket::error)?;
        }
        Ok(())
    }
}
