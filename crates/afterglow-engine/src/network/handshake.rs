use crate::network::{
    DeliveryMode, DisconnectReason, NetChannel, NetworkPacket, NetworkTransport, PeerId,
    ProtocolVersion, TransportEvent,
    session::{NetworkSession, PlatformIdentity},
};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Resource, Clone, Debug, Eq, PartialEq, Reflect)]
pub struct NetworkHandshakeConfig {
    pub protocol: ProtocolVersion,
    pub build_hash: String,
    pub content_hash: String,
    pub backend: NetworkBackendKind,
    pub identity: PlatformIdentity,
}

impl Default for NetworkHandshakeConfig {
    fn default() -> Self {
        Self {
            protocol: ProtocolVersion::CURRENT,
            build_hash: "dev".into(),
            content_hash: "dev".into(),
            backend: NetworkBackendKind::Memory,
            identity: PlatformIdentity::Anonymous {
                label: "local".into(),
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Reflect, Serialize, Deserialize)]
pub enum NetworkBackendKind {
    Memory,
    Iroh,
    Steam,
    Custom(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ControlMessage {
    Hello(ControlHello),
    Accepted(ControlAccepted),
    Rejected(ControlReject),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ControlHello {
    pub protocol: ProtocolVersion,
    pub build_hash: String,
    pub content_hash: String,
    pub backend: NetworkBackendKind,
    pub identity: PlatformIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ControlAccepted {
    pub protocol: ProtocolVersion,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ControlReject {
    pub reason: HandshakeRejectReason,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HandshakeRejectReason {
    InvalidControlPayload,
    ProtocolMismatch {
        expected: ProtocolVersion,
        got: ProtocolVersion,
    },
    BuildMismatch,
    ContentMismatch,
    DuplicateIdentity,
    PeerIdentityChanged,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HandshakeReport {
    pub sent_hellos: Vec<PeerId>,
    pub accepted_peers: Vec<PeerId>,
    pub rejected_peers: Vec<(PeerId, HandshakeRejectReason)>,
    pub disconnected_peers: Vec<PeerId>,
    pub dropped_unauthorized_packets: usize,
}

impl NetworkHandshakeConfig {
    pub fn hello(&self) -> ControlMessage {
        ControlMessage::Hello(ControlHello {
            protocol: self.protocol,
            build_hash: self.build_hash.clone(),
            content_hash: self.content_hash.clone(),
            backend: self.backend.clone(),
            identity: self.identity.clone(),
        })
    }
}

pub fn encode_control_message(message: &ControlMessage) -> Vec<u8> {
    serde_json::to_vec(message).expect("control messages are serializable")
}

pub fn decode_control_message(payload: &[u8]) -> Result<ControlMessage, serde_json::Error> {
    serde_json::from_slice(payload)
}

pub fn service_control_handshake<T: NetworkTransport>(
    transport: &mut T,
    session: &mut NetworkSession,
    config: &NetworkHandshakeConfig,
    app_events: &mut Vec<TransportEvent>,
) -> HandshakeReport {
    let mut report = HandshakeReport::default();
    let mut events = Vec::new();
    let mut blocked_peers = Vec::new();
    transport.poll_events(&mut events);

    for event in events {
        match event {
            TransportEvent::Connected(peer) => {
                send_control(transport, peer, &config.hello());
                report.sent_hellos.push(peer);
            }
            TransportEvent::Disconnected { peer, reason } => {
                session.disconnect_peer(peer);
                blocked_peers.push(peer);
                report.disconnected_peers.push(peer);
                app_events.push(TransportEvent::Disconnected { peer, reason });
            }
            TransportEvent::Packet(packet) if packet.header.channel == NetChannel::Control => {
                if blocked_peers.contains(&packet.from) {
                    report.dropped_unauthorized_packets += 1;
                    continue;
                }
                let rejected_before = report.rejected_peers.len();
                handle_control_packet(transport, session, config, packet, app_events, &mut report);
                if report.rejected_peers.len() > rejected_before
                    && let Some((peer, _)) = report.rejected_peers.last()
                {
                    blocked_peers.push(*peer);
                }
            }
            TransportEvent::Packet(packet) => {
                if blocked_peers.contains(&packet.from) {
                    report.dropped_unauthorized_packets += 1;
                } else if packet.header.protocol != config.protocol {
                    reject_peer(
                        transport,
                        session,
                        packet.from,
                        HandshakeRejectReason::ProtocolMismatch {
                            expected: config.protocol,
                            got: packet.header.protocol,
                        },
                        &mut report,
                    );
                    blocked_peers.push(packet.from);
                } else if session.peer(packet.from).is_some() {
                    app_events.push(TransportEvent::Packet(packet));
                } else {
                    report.dropped_unauthorized_packets += 1;
                }
            }
        }
    }

    report
}

fn handle_control_packet<T: NetworkTransport>(
    transport: &mut T,
    session: &mut NetworkSession,
    config: &NetworkHandshakeConfig,
    packet: NetworkPacket,
    app_events: &mut Vec<TransportEvent>,
    report: &mut HandshakeReport,
) {
    if packet.header.protocol != config.protocol {
        reject_peer(
            transport,
            session,
            packet.from,
            HandshakeRejectReason::ProtocolMismatch {
                expected: config.protocol,
                got: packet.header.protocol,
            },
            report,
        );
        return;
    }

    let Ok(message) = decode_control_message(&packet.payload) else {
        reject_peer(
            transport,
            session,
            packet.from,
            HandshakeRejectReason::InvalidControlPayload,
            report,
        );
        return;
    };

    match message {
        ControlMessage::Hello(hello) => accept_or_reject_hello(
            transport,
            session,
            config,
            packet.from,
            hello,
            app_events,
            report,
        ),
        ControlMessage::Accepted(accepted) => {
            if accepted.protocol != config.protocol {
                reject_peer(
                    transport,
                    session,
                    packet.from,
                    HandshakeRejectReason::ProtocolMismatch {
                        expected: config.protocol,
                        got: accepted.protocol,
                    },
                    report,
                );
            } else if session.peer(packet.from).is_none() {
                reject_peer(
                    transport,
                    session,
                    packet.from,
                    HandshakeRejectReason::InvalidControlPayload,
                    report,
                );
            }
        }
        ControlMessage::Rejected(reject) => {
            session.disconnect_peer(packet.from);
            report.rejected_peers.push((packet.from, reject.reason));
            transport.disconnect(packet.from, DisconnectReason::Remote);
        }
    }
}

fn accept_or_reject_hello<T: NetworkTransport>(
    transport: &mut T,
    session: &mut NetworkSession,
    config: &NetworkHandshakeConfig,
    peer: PeerId,
    hello: ControlHello,
    app_events: &mut Vec<TransportEvent>,
    report: &mut HandshakeReport,
) {
    if hello.protocol != config.protocol {
        reject_peer(
            transport,
            session,
            peer,
            HandshakeRejectReason::ProtocolMismatch {
                expected: config.protocol,
                got: hello.protocol,
            },
            report,
        );
        return;
    }
    if hello.build_hash != config.build_hash {
        reject_peer(
            transport,
            session,
            peer,
            HandshakeRejectReason::BuildMismatch,
            report,
        );
        return;
    }
    if hello.content_hash != config.content_hash {
        reject_peer(
            transport,
            session,
            peer,
            HandshakeRejectReason::ContentMismatch,
            report,
        );
        return;
    }
    if session
        .peer_for_platform(&hello.identity)
        .is_some_and(|existing| existing != peer)
    {
        reject_peer(
            transport,
            session,
            peer,
            HandshakeRejectReason::DuplicateIdentity,
            report,
        );
        return;
    }
    if let Some(existing) = session.peer(peer) {
        if existing.platform == hello.identity {
            send_control(
                transport,
                peer,
                &ControlMessage::Accepted(ControlAccepted {
                    protocol: config.protocol,
                }),
            );
        } else {
            reject_peer(
                transport,
                session,
                peer,
                HandshakeRejectReason::PeerIdentityChanged,
                report,
            );
        }
        return;
    }

    let was_new = session.connect_peer(peer, hello.identity);
    send_control(
        transport,
        peer,
        &ControlMessage::Accepted(ControlAccepted {
            protocol: config.protocol,
        }),
    );
    if was_new {
        report.accepted_peers.push(peer);
        app_events.push(TransportEvent::Connected(peer));
    }
}

fn reject_peer<T: NetworkTransport>(
    transport: &mut T,
    session: &mut NetworkSession,
    peer: PeerId,
    reason: HandshakeRejectReason,
    report: &mut HandshakeReport,
) {
    session.disconnect_peer(peer);
    send_control(
        transport,
        peer,
        &ControlMessage::Rejected(ControlReject {
            reason: reason.clone(),
        }),
    );
    transport.disconnect(peer, disconnect_reason_for_reject(&reason));
    report.rejected_peers.push((peer, reason));
}

fn disconnect_reason_for_reject(reason: &HandshakeRejectReason) -> DisconnectReason {
    match reason {
        HandshakeRejectReason::ProtocolMismatch { expected, got } => {
            DisconnectReason::ProtocolMismatch {
                expected: *expected,
                got: *got,
            }
        }
        HandshakeRejectReason::InvalidControlPayload
        | HandshakeRejectReason::BuildMismatch
        | HandshakeRejectReason::ContentMismatch
        | HandshakeRejectReason::DuplicateIdentity
        | HandshakeRejectReason::PeerIdentityChanged => DisconnectReason::Remote,
    }
}

fn send_control<T: NetworkTransport>(transport: &mut T, peer: PeerId, message: &ControlMessage) {
    transport.send(
        peer,
        NetChannel::Control,
        DeliveryMode::Reliable,
        encode_control_message(message),
    );
}

#[cfg(test)]
mod tests;
