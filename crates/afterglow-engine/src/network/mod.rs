use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, VecDeque};

pub mod authority;
pub mod baseline;
pub mod commands;
pub mod handshake;
pub mod interest;
pub mod interpolation;
#[cfg(all(feature = "iroh", not(target_arch = "wasm32")))]
pub mod iroh;
pub mod local_server;
pub mod prediction;
pub mod reconciliation;
pub mod replication;
pub mod rollback;
pub mod session;
#[cfg(all(feature = "steam", not(target_arch = "wasm32")))]
pub mod steam;

#[derive(Resource, Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub struct NetworkProtocol {
    pub version: ProtocolVersion,
}

impl Default for NetworkProtocol {
    fn default() -> Self {
        Self {
            version: ProtocolVersion::CURRENT,
        }
    }
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    Reflect,
    Serialize,
    Deserialize,
)]
pub struct PeerId(pub u64);

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    Reflect,
    Serialize,
    Deserialize,
)]
pub struct NetworkPlayerId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect, Serialize, Deserialize)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl ProtocolVersion {
    pub const CURRENT: Self = Self {
        major: 0,
        minor: 1,
        patch: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Reflect, Serialize, Deserialize)]
pub enum NetChannel {
    Control,
    Commands,
    Snapshots,
    Events,
    Bulk,
    Custom(u16),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect, Serialize, Deserialize)]
pub enum DeliveryMode {
    Reliable,
    Unreliable,
    UnreliableSequenced,
}

#[derive(Clone, Debug, Eq, PartialEq, Reflect, Serialize, Deserialize)]
pub struct PacketHeader {
    pub protocol: ProtocolVersion,
    pub channel: NetChannel,
    pub delivery: DeliveryMode,
    pub sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Reflect)]
pub struct NetworkPacket {
    pub from: PeerId,
    pub to: PeerId,
    pub header: PacketHeader,
    pub payload: Vec<u8>,
}

#[cfg(any(feature = "iroh", feature = "steam"))]
#[derive(Clone, Serialize, Deserialize)]
struct TransportWirePacket {
    header: PacketHeader,
    payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Reflect)]
pub enum DisconnectReason {
    Local,
    Remote,
    Timeout,
    ProtocolMismatch {
        expected: ProtocolVersion,
        got: ProtocolVersion,
    },
    Transport(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Reflect)]
pub enum TransportEvent {
    Connected(PeerId),
    Disconnected {
        peer: PeerId,
        reason: DisconnectReason,
    },
    Packet(NetworkPacket),
}

pub trait NetworkTransport {
    fn local_peer(&self) -> PeerId;
    fn poll_events(&mut self, out: &mut Vec<TransportEvent>);
    fn send(&mut self, to: PeerId, channel: NetChannel, delivery: DeliveryMode, payload: Vec<u8>);
    fn disconnect(&mut self, peer: PeerId, reason: DisconnectReason);
}

pub struct AfterglowNetworkPlugin;

impl Plugin for AfterglowNetworkPlugin {
    fn build(&self, app: &mut App) {
        replication::configure_replication_sets(app);
        app.init_resource::<NetworkProtocol>()
            .init_resource::<baseline::ReconnectBaselineStore>()
            .init_resource::<handshake::NetworkHandshakeConfig>()
            .init_resource::<rollback::DeterministicRollbackBuffer>()
            .init_resource::<authority::ServerCommandBuffer>()
            .init_resource::<prediction::ClientPredictionBuffer>()
            .init_resource::<reconciliation::ClientReconciliationQueue>()
            .init_resource::<interpolation::RemoteInterpolationBuffer>()
            .init_resource::<interest::InterestMap>()
            .init_resource::<local_server::LocalServerConfig>()
            .init_resource::<local_server::LocalServerState>()
            .init_resource::<replication::RollbackReplicationClock>()
            .init_resource::<session::NetworkSession>()
            .add_systems(
                Update,
                authority::clear_server_command_buffer
                    .in_set(crate::core::schedule::AfterglowSet::BuildCommands),
            )
            .add_systems(
                Update,
                reconciliation::clear_reconciliation_queue
                    .in_set(crate::core::schedule::AfterglowSet::BuildCommands),
            )
            .add_systems(
                Update,
                (
                    local_server::sync_local_server_session,
                    local_server::submit_local_player_commands,
                )
                    .chain()
                    .in_set(crate::core::schedule::AfterglowSet::BuildCommands)
                    .after(authority::clear_server_command_buffer),
            );
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FaultConfig {
    pub drop_every: Option<u64>,
    pub duplicate_every: Option<u64>,
    pub delay_ticks: u32,
    pub reverse_delivery: bool,
}

#[derive(Clone, Debug)]
pub struct MemoryTransport {
    local_peer: PeerId,
    protocol: ProtocolVersion,
    connected: BTreeSet<PeerId>,
    incoming: VecDeque<TransportEvent>,
    outgoing: VecDeque<NetworkPacket>,
    delayed: Vec<ScheduledEvent>,
    delivered_sequences: HashMap<(PeerId, NetChannel), u64>,
    next_sequence: u64,
    faults: FaultConfig,
}

#[derive(Clone, Debug)]
struct ScheduledEvent {
    remaining_ticks: u32,
    event: TransportEvent,
}

impl MemoryTransport {
    pub fn new(local_peer: PeerId) -> Self {
        Self {
            local_peer,
            protocol: ProtocolVersion::CURRENT,
            connected: BTreeSet::new(),
            incoming: VecDeque::new(),
            outgoing: VecDeque::new(),
            delayed: Vec::new(),
            delivered_sequences: HashMap::new(),
            next_sequence: 0,
            faults: FaultConfig::default(),
        }
    }

    pub fn with_faults(mut self, faults: FaultConfig) -> Self {
        self.faults = faults;
        self
    }

    pub fn with_protocol(mut self, protocol: ProtocolVersion) -> Self {
        self.protocol = protocol;
        self
    }

    pub fn connect_pair(a: PeerId, b: PeerId) -> (Self, Self) {
        let mut left = Self::new(a);
        let mut right = Self::new(b);
        left.connect_peer(b);
        right.connect_peer(a);
        (left, right)
    }

    pub fn connect_peer(&mut self, peer: PeerId) {
        if self.connected.insert(peer) {
            self.delivered_sequences
                .retain(|(from, _), _| *from != peer);
            self.incoming.push_back(TransportEvent::Connected(peer));
        }
    }

    pub fn pump_pair(left: &mut Self, right: &mut Self) {
        let left_packets = left.collect_outgoing();
        let right_packets = right.collect_outgoing();
        right.receive_packets(left_packets);
        left.receive_packets(right_packets);
        left.advance_tick();
        right.advance_tick();
    }

    pub fn collect_outgoing(&mut self) -> Vec<NetworkPacket> {
        self.outgoing.drain(..).collect()
    }

    pub fn receive_packets(&mut self, mut packets: Vec<NetworkPacket>) {
        if self.faults.reverse_delivery {
            packets.reverse();
        }
        for packet in packets {
            if packet.to != self.local_peer || !self.connected.contains(&packet.from) {
                continue;
            }
            if self.should_drop(packet.header.sequence) {
                continue;
            }
            let duplicate = self.should_duplicate(packet.header.sequence);
            self.schedule(TransportEvent::Packet(packet.clone()));
            if duplicate {
                self.schedule(TransportEvent::Packet(packet));
            }
        }
    }

    fn should_drop(&self, sequence: u64) -> bool {
        self.faults
            .drop_every
            .is_some_and(|every| every > 0 && (sequence + 1).is_multiple_of(every))
    }

    fn should_duplicate(&self, sequence: u64) -> bool {
        self.faults
            .duplicate_every
            .is_some_and(|every| every > 0 && (sequence + 1).is_multiple_of(every))
    }

    fn schedule(&mut self, event: TransportEvent) {
        if self.faults.delay_ticks == 0 {
            self.incoming.push_back(event);
            return;
        }
        self.delayed.push(ScheduledEvent {
            remaining_ticks: self.faults.delay_ticks,
            event,
        });
    }

    pub fn advance_tick(&mut self) {
        let mut ready = Vec::new();
        for delayed in &mut self.delayed {
            delayed.remaining_ticks = delayed.remaining_ticks.saturating_sub(1);
        }
        self.delayed.retain(|delayed| {
            if delayed.remaining_ticks == 0 {
                ready.push(delayed.event.clone());
                false
            } else {
                true
            }
        });
        self.incoming.extend(ready);
    }
}

impl NetworkTransport for MemoryTransport {
    fn local_peer(&self) -> PeerId {
        self.local_peer
    }

    fn poll_events(&mut self, out: &mut Vec<TransportEvent>) {
        let events = self.incoming.drain(..).collect::<Vec<_>>();
        for event in events {
            if let TransportEvent::Packet(packet) = &event
                && !accepts_unreliable_sequence(&mut self.delivered_sequences, packet)
            {
                continue;
            }
            out.push(event);
        }
    }

    fn send(&mut self, to: PeerId, channel: NetChannel, delivery: DeliveryMode, payload: Vec<u8>) {
        if !self.connected.contains(&to) {
            return;
        }
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.outgoing.push_back(NetworkPacket {
            from: self.local_peer,
            to,
            header: PacketHeader {
                protocol: self.protocol,
                channel,
                delivery,
                sequence,
            },
            payload,
        });
    }

    fn disconnect(&mut self, peer: PeerId, reason: DisconnectReason) {
        if self.connected.remove(&peer) {
            self.delivered_sequences
                .retain(|(from, _), _| *from != peer);
            self.incoming
                .push_back(TransportEvent::Disconnected { peer, reason });
        }
    }
}

pub(super) fn accepts_unreliable_sequence(
    delivered_sequences: &mut HashMap<(PeerId, NetChannel), u64>,
    packet: &NetworkPacket,
) -> bool {
    if packet.header.delivery != DeliveryMode::UnreliableSequenced {
        return true;
    }
    let key = (packet.from, packet.header.channel);
    let sequence = packet.header.sequence;
    if delivered_sequences
        .get(&key)
        .is_some_and(|delivered| sequence <= *delivered)
    {
        return false;
    }
    delivered_sequences.insert(key, sequence);
    true
}

#[cfg(any(feature = "iroh", feature = "steam"))]
pub(super) fn encode_transport_packet(
    packet: &NetworkPacket,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&TransportWirePacket {
        header: packet.header.clone(),
        payload: packet.payload.clone(),
    })
}

#[cfg(any(feature = "iroh", feature = "steam"))]
pub(super) fn decode_transport_packet(
    from: PeerId,
    to: PeerId,
    bytes: &[u8],
) -> Option<NetworkPacket> {
    let wire = serde_json::from_slice::<TransportWirePacket>(bytes).ok()?;
    Some(NetworkPacket {
        from,
        to,
        header: wire.header,
        payload: wire.payload,
    })
}

#[cfg(test)]
mod tests;
