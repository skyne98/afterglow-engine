use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};

use bevy::prelude::*;

use crate::network::session::{
    non_steam::{
        handle_create, handle_join, handle_join_by_code, handle_leave, handle_search,
        NonSteamSessionCatalog,
    },
    AfterglowSessionState, SessionBackend, SessionError, SessionEvent, SessionIdentityNonce,
    SessionMemberId, SessionRequest, SessionId,
};

use super::protocol;

/// In-memory NonSteam session provider that listens for control-plane requests
/// over TCP.
///
/// Each accepted connection is treated as a remote client. The provider hosts
/// its own `NonSteamSessionCatalog`, validates identity proofs with the global
/// `SessionIdentityNonce`, and replies with `SessionEvent`s.
#[derive(Resource, Debug)]
pub struct NonSteamSessionProvider {
    pub listen_addr: SocketAddr,
    pub(crate) listener: Option<TcpListener>,
    pub(crate) catalog: NonSteamSessionCatalog,
    pub(crate) clients: Vec<ProviderClient>,
}

impl NonSteamSessionProvider {
    /// Start listening on the given address.
    pub fn new(listen_addr: SocketAddr) -> io::Result<Self> {
        let listener = TcpListener::bind(listen_addr)?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listen_addr,
            listener: Some(listener),
            catalog: NonSteamSessionCatalog::default(),
            clients: Vec::new(),
        })
    }
}

impl Default for NonSteamSessionProvider {
    fn default() -> Self {
        Self {
            listen_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 0),
            listener: None,
            catalog: NonSteamSessionCatalog::default(),
            clients: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct ProviderClient {
    pub(crate) socket: TcpStream,
    pub(crate) read_buf: Vec<u8>,
    pub(crate) write_buf: Vec<u8>,
    pub(crate) member_id: Option<SessionMemberId>,
    pub(crate) session_id: Option<SessionId>,
}

impl ProviderClient {
    fn new(socket: TcpStream) -> io::Result<Self> {
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            read_buf: Vec::new(),
            write_buf: Vec::new(),
            member_id: None,
            session_id: None,
        })
    }

    fn try_read_request(&mut self) -> io::Result<Option<SessionRequest>> {
        let mut tmp = [0u8; 1024];
        match self.socket.read(&mut tmp) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "client closed")),
            Ok(n) => self.read_buf.extend_from_slice(&tmp[..n]),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(None),
            Err(e) => return Err(e),
        }

        match protocol::try_pop_request(&mut self.read_buf) {
            Ok(Some(req)) => Ok(Some(req)),
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn queue_event(&mut self, event: &SessionEvent) -> io::Result<()> {
        protocol::write_event(&mut self.write_buf, event)
    }

    fn flush_write(&mut self) -> io::Result<()> {
        let mut total = 0;
        while total < self.write_buf.len() {
            match self.socket.write(&self.write_buf[total..]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "write returned 0",
                    ))
                }
                Ok(n) => total += n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
        self.write_buf.drain(..total);
        Ok(())
    }
}

/// Bevy system that drives the provider listener and client sockets.
pub(crate) fn run_non_steam_provider(
    mut provider: ResMut<NonSteamSessionProvider>,
    nonce: Res<SessionIdentityNonce>,
) {
    accept_new_clients(&mut provider);

    let provider = &mut *provider;
    let (catalog, clients) = (&mut provider.catalog, &mut provider.clients);

    for i in (0..clients.len()).rev() {
        let events = {
            let client = &mut clients[i];

            if let Err(e) = client.flush_write()
                && e.kind() != io::ErrorKind::WouldBlock
            {
                clients.remove(i);
                continue;
            }

            match client.try_read_request() {
                Ok(Some(request)) => {
                    Some(handle_client_request(catalog, client, &request, &nonce.0))
                }
                Ok(None) => None,
                Err(_) => {
                    clients.remove(i);
                    continue;
                }
            }
        };

        if let Some(events) = events {
            broadcast_provider_events(i, clients, &events);
        }

        let client = &mut clients[i];
        if let Err(e) = client.flush_write()
            && e.kind() != io::ErrorKind::WouldBlock
        {
            clients.remove(i);
        }
    }
}

fn handle_client_request(
    catalog: &mut NonSteamSessionCatalog,
    client: &mut ProviderClient,
    request: &SessionRequest,
    nonce: &[u8; 32],
) -> Vec<SessionEvent> {
    let mut events = Vec::new();
    handle_remote_request(catalog, client, request, nonce, &mut events);
    for event in &events {
        if let Err(e) = client.queue_event(event) {
            bevy::log::warn!("failed to queue event for remote client: {:?}", e);
        }
    }
    events
}

fn accept_new_clients(provider: &mut NonSteamSessionProvider) {
    let listener = match provider.listener.as_ref() {
        Some(l) => l,
        None => return,
    };

    loop {
        match listener.accept() {
            Ok((socket, _addr)) => {
                if let Ok(client) = ProviderClient::new(socket) {
                    provider.clients.push(client);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(e) => {
                bevy::log::warn!("provider accept error: {:?}", e);
                break;
            }
        }
    }
}

fn handle_remote_request(
    catalog: &mut NonSteamSessionCatalog,
    client: &mut ProviderClient,
    request: &SessionRequest,
    nonce: &[u8; 32],
    events: &mut Vec<SessionEvent>,
) {
    let mut state = AfterglowSessionState::default();
    if let Some(member) = client.member_id {
        state.local_member_id = member;
    }
    if let Some(session) = client.session_id {
        state.current_session = Some(session);
        state.current_backend = Some(SessionBackend::NonSteam);
    }

    match request {
        SessionRequest::Create(config, identity) => {
            if !validate_backend(&config.backend, events) {
                return;
            }
            handle_create(config, identity, nonce, catalog, &mut state, events);
        }
        SessionRequest::Search(search) => {
            if !validate_backend(&search.backend, events) {
                return;
            }
            handle_search(search, catalog, events);
        }
        SessionRequest::Join {
            backend,
            session,
            identity,
            ..
        } => {
            if !validate_backend(backend, events) {
                return;
            }
            let target = session.as_raw().to_string();
            handle_join(
                *session, &target, identity, nonce, catalog, &mut state, events,
            );
        }
        SessionRequest::JoinByCode {
            backend,
            code,
            identity,
            ..
        } => {
            if !validate_backend(backend, events) {
                return;
            }
            handle_join_by_code(code.clone(), identity, nonce, catalog, &mut state, events);
        }
        SessionRequest::Leave => {
            handle_leave(catalog, &mut state, events);
        }
    }

    // Track the remote client's session membership for future requests.
    if state.current_session.is_some() {
        client.session_id = state.current_session;
        client.member_id = Some(state.local_member_id);
    } else if client.session_id.is_some() {
        // Leave / SessionEnded cleared state.
        client.session_id = None;
        client.member_id = None;
    }
}

/// Broadcasts `MemberJoined`, `MemberLeft`, and `SessionEnded` events to all
/// provider clients in the affected session except the requester itself
/// (identified by `requester_index`).
pub(crate) fn broadcast_provider_events(
    requester_index: usize,
    clients: &mut [ProviderClient],
    events: &[SessionEvent],
) {
    for event in events {
        let session_id = match event {
            SessionEvent::MemberJoined { session, .. } => Some(*session),
            SessionEvent::MemberLeft { session, .. } => Some(*session),
            SessionEvent::SessionEnded(session) => Some(*session),
            _ => continue,
        };

        let (before, after) = clients.split_at_mut(requester_index);
        let (_requester, after) = after
            .split_first_mut()
            .expect("requester index must be valid");

        for client in before.iter_mut().chain(after.iter_mut()) {
            if client.session_id == session_id
                && let Err(e) = client.queue_event(event)
            {
                bevy::log::warn!("failed to broadcast event to remote client: {:?}", e);
            }
        }
    }
}

fn validate_backend(
    backend: &SessionBackend,
    events: &mut Vec<SessionEvent>,
) -> bool {
    match backend {
        SessionBackend::NonSteam => true,
        SessionBackend::Steam => {
            events.push(SessionEvent::Error(SessionError::BackendUnavailable));
            false
        }
    }
}
