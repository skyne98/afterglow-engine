use std::io::{self, Read};
use std::net::TcpStream;

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
    /// written; responses appear as `SessionEvent` Bevy messages on subsequent
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

        if self.connection.is_none() {
            let stream = TcpStream::connect(addr)?;
            stream.set_nonblocking(true)?;
            self.connection = Some(ClientConnection {
                socket: stream,
                read_buf: Vec::new(),
            });
        }

        if let Some(conn) = &mut self.connection {
            protocol::write_request(&mut conn.socket, request)?;
        }

        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct ClientConnection {
    pub(crate) socket: TcpStream,
    pub(crate) read_buf: Vec<u8>,
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
