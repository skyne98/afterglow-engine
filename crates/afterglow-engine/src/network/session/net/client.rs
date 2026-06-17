use std::{
    io::{self, Read, Write},
    net::TcpStream,
};

use bevy::prelude::*;

use crate::network::session::{ProviderEndpoint, SessionEvent, SessionRequest};

use super::protocol;

/// Client that connects to a remote `NonSteamSessionProvider` over TCP and
/// surfaces the responses as local `SessionEvent` messages.
#[derive(Resource, Debug, Default)]
pub struct NonSteamSessionClient {
    /// Active connection, if any.
    pub(crate) connection: Option<ClientConnection>,
}

impl NonSteamSessionClient {
    /// Send a request to the provider at the given endpoint.
    ///
    /// If no connection exists, one is opened. The request is serialized and
    /// queued; responses appear as `SessionEvent` Bevy messages on subsequent
    /// frames.
    pub fn send_request(
        &mut self,
        endpoint: &ProviderEndpoint,
        request: &SessionRequest,
    ) -> io::Result<()> {
        let addr = match endpoint {
            ProviderEndpoint::Udp(addr) => *addr,
            ProviderEndpoint::InProcess | ProviderEndpoint::Steam => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "NonSteamSessionClient only supports Udp providers",
                ));
            }
        };

        let needs_reconnect = match &self.connection {
            Some(conn) => conn.peer != addr,
            None => true,
        };
        if needs_reconnect {
            let stream = TcpStream::connect(addr)?;
            stream.set_nonblocking(true)?;
            self.connection = Some(ClientConnection {
                socket: stream,
                read_buf: Vec::new(),
                write_buf: Vec::new(),
                peer: addr,
            });
        }

        let bytes = protocol::encode_request(request)?;
        if let Some(conn) = &mut self.connection {
            // Buffer the request; the non-blocking socket may not accept the
            // full payload in one write. The poll system flushes any pending
            // data each frame.
            conn.write_buf.extend_from_slice(&bytes);
        }
        Ok(())
    }

    /// Close the active connection, if any. Safe to call at any time.
    pub fn disconnect(&mut self) {
        self.connection = None;
    }
}

#[derive(Debug)]
pub(crate) struct ClientConnection {
    pub(crate) socket: TcpStream,
    pub(crate) read_buf: Vec<u8>,
    pub(crate) write_buf: Vec<u8>,
    pub(crate) peer: std::net::SocketAddr,
}

/// Bevy system that polls the client socket and writes received events into
/// the local `SessionEvent` message channel.
pub(crate) fn poll_non_steam_client(
    mut client: ResMut<NonSteamSessionClient>,
    mut messages: MessageWriter<SessionEvent>,
) {
    let conn = match client.connection.as_mut() {
        Some(c) => c,
        None => return,
    };

    // Flush any pending write data first.
    if !conn.write_buf.is_empty() {
        match conn.socket.write(&conn.write_buf) {
            Ok(0) => {
                client.connection = None;
                return;
            }
            Ok(n) => {
                conn.write_buf.drain(..n);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(_) => {
                client.connection = None;
                return;
            }
        }
    }

    let mut tmp = [0u8; 1024];
    match conn.socket.read(&mut tmp) {
        Ok(0) => {
            client.connection = None;
            return;
        }
        Ok(n) => conn.read_buf.extend_from_slice(&tmp[..n]),
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => return,
        Err(_) => {
            client.connection = None;
            return;
        }
    }

    loop {
        match protocol::try_pop_event(&mut conn.read_buf) {
            Ok(Some(event)) => {
                messages.write(event);
            }
            Ok(None) => break,
            Err(_) => {
                client.connection = None;
                return;
            }
        }
    }
}
