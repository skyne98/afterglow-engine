use crate::network::{
    DeliveryMode, DisconnectReason, NetChannel, NetworkPacket, NetworkTransport, PacketHeader,
    PeerId, ProtocolVersion, TransportEvent, accepts_unreliable_sequence, decode_transport_packet,
    encode_transport_packet,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use steamworks::{
    Client, LobbyId, SteamId,
    networking_sockets::{ListenSocket, NetConnection, NetworkingSockets},
    networking_types::{
        AppNetConnectionEnd, ConnectedEvent, DisconnectedEvent, ListenSocketEvent,
        NetConnectionEnd, NetworkingConfigEntry, NetworkingConfigValue, NetworkingConnectionState,
        NetworkingIdentity, SendFlags,
    },
};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SteamTransportConfig {
    pub protocol: ProtocolVersion,
    pub local_virtual_port: i32,
    pub next_inbound_peer: u64,
    pub max_messages_per_poll: usize,
    pub listen: bool,
    pub init_relay_access: bool,
}

impl Default for SteamTransportConfig {
    fn default() -> Self {
        Self {
            protocol: ProtocolVersion::CURRENT,
            local_virtual_port: 0,
            next_inbound_peer: 1,
            max_messages_per_poll: 256,
            listen: true,
            init_relay_access: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SteamLobbyMetadata {
    pub protocol: ProtocolVersion,
    pub build_hash: String,
    pub content_hash: String,
    pub world_id: String,
    pub host_steam_id: u64,
    pub host_virtual_port: i32,
}

impl SteamLobbyMetadata {
    pub fn entries(&self) -> [(&'static str, String); 6] {
        [
            ("protocol", protocol_key(self.protocol)),
            ("build_hash", self.build_hash.clone()),
            ("content_hash", self.content_hash.clone()),
            ("world_id", self.world_id.clone()),
            ("host_steam_id", self.host_steam_id.to_string()),
            ("host_virtual_port", self.host_virtual_port.to_string()),
        ]
    }
}

#[derive(Debug, Error)]
pub enum SteamTransportError {
    #[error("failed to initialize Steam client: {0}")]
    ClientInit(String),
    #[error("failed to create Steam P2P listen socket")]
    ListenSocket,
}

struct SteamConnection {
    steam_id: Option<u64>,
    connected: bool,
    connection: NetConnection,
}

pub struct SteamTransport {
    local_peer: PeerId,
    protocol: ProtocolVersion,
    local_virtual_port: i32,
    next_sequence: u64,
    next_inbound_peer: u64,
    delivered_sequences: HashMap<(PeerId, NetChannel), u64>,
    client: Client,
    sockets: NetworkingSockets,
    listen_socket: Option<ListenSocket>,
    connections: HashMap<PeerId, SteamConnection>,
    peer_by_steam: HashMap<u64, PeerId>,
    pending_events: Vec<TransportEvent>,
    max_messages_per_poll: usize,
}

impl SteamTransport {
    pub fn new(
        local_peer: PeerId,
        config: SteamTransportConfig,
    ) -> Result<Self, SteamTransportError> {
        let client =
            Client::init().map_err(|err| SteamTransportError::ClientInit(err.to_string()))?;
        if config.init_relay_access {
            client.networking_utils().init_relay_network_access();
        }
        let sockets = client.networking_sockets();
        let listen_socket = if config.listen {
            Some(
                sockets
                    .create_listen_socket_p2p(config.local_virtual_port, steam_options())
                    .map_err(|_| SteamTransportError::ListenSocket)?,
            )
        } else {
            None
        };
        Ok(Self {
            local_peer,
            protocol: config.protocol,
            local_virtual_port: config.local_virtual_port,
            next_sequence: 0,
            next_inbound_peer: config.next_inbound_peer,
            delivered_sequences: HashMap::new(),
            client,
            sockets,
            listen_socket,
            connections: HashMap::new(),
            peer_by_steam: HashMap::new(),
            pending_events: Vec::new(),
            max_messages_per_poll: config.max_messages_per_poll.max(1),
        })
    }

    pub fn local_steam_id(&self) -> u64 {
        self.client.user().steam_id().raw()
    }

    pub fn connect_steam(&mut self, peer: PeerId, steam_id: u64, remote_virtual_port: i32) {
        let identity = NetworkingIdentity::new_steam_id(SteamId::from_raw(steam_id));
        let Ok(connection) =
            self.sockets
                .connect_p2p(identity, remote_virtual_port, steam_options())
        else {
            self.pending_events.push(TransportEvent::Disconnected {
                peer,
                reason: DisconnectReason::Transport("Steam P2P connect failed".into()),
            });
            return;
        };
        self.insert_connection(peer, Some(steam_id), false, connection);
    }

    pub fn lobby_metadata(
        &self,
        build_hash: &str,
        content_hash: &str,
        world_id: &str,
    ) -> SteamLobbyMetadata {
        SteamLobbyMetadata {
            protocol: self.protocol,
            build_hash: build_hash.into(),
            content_hash: content_hash.into(),
            world_id: world_id.into(),
            host_steam_id: self.local_steam_id(),
            host_virtual_port: self.local_virtual_port,
        }
    }

    pub fn lobby_identity(lobby: LobbyId) -> u64 {
        lobby.raw()
    }

    fn poll_steam(&mut self, out: &mut Vec<TransportEvent>) {
        self.client.run_callbacks();
        self.poll_listen_socket(out);
        self.poll_connection_events(out);
        self.poll_messages(out);
        for event in self.pending_events.drain(..) {
            out.push(event);
        }
    }

    fn poll_listen_socket(&mut self, out: &mut Vec<TransportEvent>) {
        while let Some(event) = self
            .listen_socket
            .as_ref()
            .and_then(ListenSocket::try_receive_event)
        {
            match event {
                ListenSocketEvent::Connecting(request) => {
                    let _ = request.accept();
                }
                ListenSocketEvent::Connected(connected) => {
                    let peer = self.peer_for_connected(&connected);
                    let steam_id = connected.remote().steam_id().map(|id| id.raw());
                    self.insert_connection(peer, steam_id, true, connected.take_connection());
                    out.push(TransportEvent::Connected(peer));
                }
                ListenSocketEvent::Disconnected(disconnected) => {
                    self.remove_disconnected(disconnected, out);
                }
            }
        }
    }

    fn poll_connection_events(&mut self, out: &mut Vec<TransportEvent>) {
        let peers = self.connections.keys().copied().collect::<Vec<_>>();
        for peer in peers {
            let Some(connection) = self.connections.get_mut(&peer) else {
                continue;
            };
            let Some(events) = connection.connection.try_events() else {
                continue;
            };
            let events = events.collect::<Vec<_>>();
            for event in events {
                match event.new_state {
                    NetworkingConnectionState::Connected if !connection.connected => {
                        connection.connected = true;
                        out.push(TransportEvent::Connected(peer));
                    }
                    NetworkingConnectionState::ClosedByPeer
                    | NetworkingConnectionState::ProblemDetectedLocally => {
                        self.remove_peer(peer, DisconnectReason::Remote, out);
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    fn poll_messages(&mut self, out: &mut Vec<TransportEvent>) {
        for peer in self.connections.keys().copied().collect::<Vec<_>>() {
            let Some(connection) = self.connections.get_mut(&peer) else {
                continue;
            };
            let Ok(messages) = connection
                .connection
                .receive_messages(self.max_messages_per_poll)
            else {
                continue;
            };
            for message in messages {
                if let Some(packet) = decode_transport_packet(peer, self.local_peer, message.data())
                    && accepts_unreliable_sequence(&mut self.delivered_sequences, &packet)
                {
                    out.push(TransportEvent::Packet(packet));
                }
            }
        }
    }

    fn peer_for_connected(&mut self, connected: &ConnectedEvent) -> PeerId {
        if let Some(steam_id) = connected.remote().steam_id().map(|id| id.raw())
            && let Some(peer) = self.peer_by_steam.get(&steam_id)
        {
            return *peer;
        }
        self.allocate_inbound_peer()
    }

    fn insert_connection(
        &mut self,
        peer: PeerId,
        steam_id: Option<u64>,
        connected: bool,
        connection: NetConnection,
    ) {
        if let Some(replaced) = self.connections.remove(&peer) {
            close_connection(replaced.connection, "connection replaced");
        }
        if let Some(steam_id) = steam_id {
            self.peer_by_steam.insert(steam_id, peer);
        }
        connection.set_connection_user_data(peer.0 as i64).ok();
        self.connections.insert(
            peer,
            SteamConnection {
                steam_id,
                connected,
                connection,
            },
        );
    }

    fn remove_disconnected(
        &mut self,
        disconnected: DisconnectedEvent,
        out: &mut Vec<TransportEvent>,
    ) {
        if let Some(steam_id) = disconnected.remote().steam_id().map(|id| id.raw())
            && let Some(peer) = self.peer_by_steam.remove(&steam_id)
        {
            self.remove_peer(peer, DisconnectReason::Remote, out);
        }
    }

    fn remove_peer(
        &mut self,
        peer: PeerId,
        reason: DisconnectReason,
        out: &mut Vec<TransportEvent>,
    ) {
        if let Some(removed) = self.connections.remove(&peer) {
            if let Some(steam_id) = removed.steam_id {
                self.peer_by_steam.remove(&steam_id);
            }
            self.delivered_sequences
                .retain(|(from, _), _| *from != peer);
            out.push(TransportEvent::Disconnected { peer, reason });
        }
    }

    fn allocate_inbound_peer(&mut self) -> PeerId {
        loop {
            let peer = PeerId(self.next_inbound_peer);
            self.next_inbound_peer = self.next_inbound_peer.saturating_add(1);
            if peer != self.local_peer && !self.connections.contains_key(&peer) {
                return peer;
            }
        }
    }
}

impl NetworkTransport for SteamTransport {
    fn local_peer(&self) -> PeerId {
        self.local_peer
    }

    fn poll_events(&mut self, out: &mut Vec<TransportEvent>) {
        self.poll_steam(out);
    }

    fn send(&mut self, to: PeerId, channel: NetChannel, delivery: DeliveryMode, payload: Vec<u8>) {
        let Some(connection) = self.connections.get(&to) else {
            return;
        };
        let packet = NetworkPacket {
            from: self.local_peer,
            to,
            header: PacketHeader {
                protocol: self.protocol,
                channel,
                delivery,
                sequence: self.next_sequence,
            },
            payload,
        };
        self.next_sequence = self.next_sequence.saturating_add(1);
        if let Ok(bytes) = encode_transport_packet(&packet) {
            let _ = connection
                .connection
                .send_message(&bytes, send_flags(delivery));
        }
    }

    fn disconnect(&mut self, peer: PeerId, reason: DisconnectReason) {
        if let Some(connection) = self.connections.remove(&peer) {
            if let Some(steam_id) = connection.steam_id {
                self.peer_by_steam.remove(&steam_id);
            }
            self.delivered_sequences
                .retain(|(from, _), _| *from != peer);
            close_connection(connection.connection, &format!("{reason:?}"));
            self.pending_events
                .push(TransportEvent::Disconnected { peer, reason });
        }
    }
}

fn send_flags(delivery: DeliveryMode) -> SendFlags {
    match delivery {
        DeliveryMode::Reliable => SendFlags::RELIABLE,
        DeliveryMode::Unreliable | DeliveryMode::UnreliableSequenced => SendFlags::UNRELIABLE,
    }
}

fn steam_options() -> Vec<NetworkingConfigEntry> {
    vec![NetworkingConfigEntry::new_int32(
        NetworkingConfigValue::SymmetricConnect,
        1,
    )]
}

fn close_connection(connection: NetConnection, debug: &str) {
    let _ = connection.close(
        NetConnectionEnd::App(AppNetConnectionEnd::generic_normal()),
        Some(debug),
        false,
    );
}

fn protocol_key(protocol: ProtocolVersion) -> String {
    format!("{}.{}.{}", protocol.major, protocol.minor, protocol.patch)
}

#[cfg(test)]
mod tests;
