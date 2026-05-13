use super::*;
use crate::network::{MemoryTransport, NetworkTransport};

mod edge;

fn config(label: &str) -> NetworkHandshakeConfig {
    NetworkHandshakeConfig {
        protocol: ProtocolVersion::CURRENT,
        build_hash: "build-a".into(),
        content_hash: "content-a".into(),
        backend: NetworkBackendKind::Memory,
        identity: PlatformIdentity::Anonymous {
            label: label.into(),
        },
    }
}

fn pump_handshake(
    left: &mut MemoryTransport,
    left_session: &mut NetworkSession,
    left_config: &NetworkHandshakeConfig,
    right: &mut MemoryTransport,
    right_session: &mut NetworkSession,
    right_config: &NetworkHandshakeConfig,
) -> (Vec<TransportEvent>, Vec<TransportEvent>) {
    let mut left_events = Vec::new();
    let mut right_events = Vec::new();
    service_control_handshake(left, left_session, left_config, &mut left_events);
    service_control_handshake(right, right_session, right_config, &mut right_events);
    MemoryTransport::pump_pair(left, right);
    service_control_handshake(left, left_session, left_config, &mut left_events);
    service_control_handshake(right, right_session, right_config, &mut right_events);
    (left_events, right_events)
}

#[test]
fn connected_peers_exchange_hellos_and_enter_session() {
    let (mut client, mut server) = MemoryTransport::connect_pair(PeerId(1), PeerId(2));
    let mut client_session = NetworkSession::default();
    let mut server_session = NetworkSession::default();
    let client_config = config("client");
    let server_config = config("server");

    let (client_events, server_events) = pump_handshake(
        &mut client,
        &mut client_session,
        &client_config,
        &mut server,
        &mut server_session,
        &server_config,
    );

    assert_eq!(
        client_session.peer(PeerId(2)).unwrap().platform,
        server_config.identity
    );
    assert_eq!(
        server_session.peer(PeerId(1)).unwrap().platform,
        client_config.identity
    );
    assert_eq!(client_events, [TransportEvent::Connected(PeerId(2))]);
    assert_eq!(server_events, [TransportEvent::Connected(PeerId(1))]);
}

#[test]
fn gameplay_packets_are_hidden_until_handshake_accepts_peer() {
    let (mut client, mut server) = MemoryTransport::connect_pair(PeerId(1), PeerId(2));
    let mut server_session = NetworkSession::default();
    let server_config = config("server");

    client.send(
        PeerId(2),
        NetChannel::Commands,
        DeliveryMode::Reliable,
        b"cmd-before-auth".to_vec(),
    );
    MemoryTransport::pump_pair(&mut client, &mut server);

    let mut app_events = Vec::new();
    let report = service_control_handshake(
        &mut server,
        &mut server_session,
        &server_config,
        &mut app_events,
    );

    assert_eq!(report.dropped_unauthorized_packets, 1);
    assert!(app_events.is_empty());
    assert!(server_session.peer(PeerId(1)).is_none());
}

#[test]
fn accepted_peer_gameplay_packets_are_forwarded() {
    let (mut client, mut server) = MemoryTransport::connect_pair(PeerId(1), PeerId(2));
    let mut client_session = NetworkSession::default();
    let mut server_session = NetworkSession::default();
    let client_config = config("client");
    let server_config = config("server");
    pump_handshake(
        &mut client,
        &mut client_session,
        &client_config,
        &mut server,
        &mut server_session,
        &server_config,
    );

    client.send(
        PeerId(2),
        NetChannel::Commands,
        DeliveryMode::Reliable,
        b"cmd-after-auth".to_vec(),
    );
    MemoryTransport::pump_pair(&mut client, &mut server);

    let mut app_events = Vec::new();
    let report = service_control_handshake(
        &mut server,
        &mut server_session,
        &server_config,
        &mut app_events,
    );

    assert_eq!(report.dropped_unauthorized_packets, 0);
    assert!(matches!(
        app_events.as_slice(),
        [TransportEvent::Packet(packet)] if packet.payload == b"cmd-after-auth"
    ));
}

#[test]
fn protocol_mismatch_rejects_peer_without_session_entry() {
    let (mut client, mut server) = MemoryTransport::connect_pair(PeerId(1), PeerId(2));
    let mut client_session = NetworkSession::default();
    let mut server_session = NetworkSession::default();
    let mut client_config = config("client");
    let server_config = config("server");
    client_config.protocol = ProtocolVersion {
        major: 9,
        minor: 0,
        patch: 0,
    };

    let (_client_events, server_events) = pump_handshake(
        &mut client,
        &mut client_session,
        &client_config,
        &mut server,
        &mut server_session,
        &server_config,
    );

    assert!(server_events.is_empty());
    assert!(server_session.peer(PeerId(1)).is_none());
    let mut events = Vec::new();
    server.poll_events(&mut events);
    assert!(events.iter().any(|event| matches!(
        event,
        TransportEvent::Disconnected {
            peer: PeerId(1),
            reason: DisconnectReason::ProtocolMismatch { got, .. },
        } if *got == ProtocolVersion {
            major: 9,
            minor: 0,
            patch: 0,
        }
    )));
}

#[test]
fn packet_header_protocol_mismatch_rejects_peer_before_payload() {
    let mut client = MemoryTransport::new(PeerId(1)).with_protocol(ProtocolVersion {
        major: 7,
        minor: 0,
        patch: 0,
    });
    let mut server = MemoryTransport::new(PeerId(2));
    client.connect_peer(PeerId(2));
    server.connect_peer(PeerId(1));
    let mut server_session = NetworkSession::default();
    let server_config = config("server");

    client.send(
        PeerId(2),
        NetChannel::Control,
        DeliveryMode::Reliable,
        encode_control_message(&server_config.hello()),
    );
    MemoryTransport::pump_pair(&mut client, &mut server);

    let mut app_events = Vec::new();
    let report = service_control_handshake(
        &mut server,
        &mut server_session,
        &server_config,
        &mut app_events,
    );

    assert_eq!(
        report.rejected_peers,
        [(
            PeerId(1),
            HandshakeRejectReason::ProtocolMismatch {
                expected: ProtocolVersion::CURRENT,
                got: ProtocolVersion {
                    major: 7,
                    minor: 0,
                    patch: 0,
                },
            },
        )]
    );
    assert!(server_session.peer(PeerId(1)).is_none());
    assert!(app_events.is_empty());
    let mut events = Vec::new();
    server.poll_events(&mut events);
    assert!(events.iter().any(|event| matches!(
        event,
        TransportEvent::Disconnected {
            peer: PeerId(1),
            reason: DisconnectReason::ProtocolMismatch { got, .. },
        } if *got == ProtocolVersion {
            major: 7,
            minor: 0,
            patch: 0,
        }
    )));
}

#[test]
fn build_or_content_mismatch_rejects_peer() {
    for client_config in [
        NetworkHandshakeConfig {
            build_hash: "other-build".into(),
            ..config("client")
        },
        NetworkHandshakeConfig {
            content_hash: "other-content".into(),
            ..config("client")
        },
    ] {
        let (mut client, mut server) = MemoryTransport::connect_pair(PeerId(1), PeerId(2));
        let mut client_session = NetworkSession::default();
        let mut server_session = NetworkSession::default();
        let server_config = config("server");

        let (_client_events, server_events) = pump_handshake(
            &mut client,
            &mut client_session,
            &client_config,
            &mut server,
            &mut server_session,
            &server_config,
        );

        assert!(server_events.is_empty());
        assert!(server_session.peer(PeerId(1)).is_none());
    }
}

#[test]
fn duplicate_external_identity_is_rejected() {
    let (mut peer, mut server) = MemoryTransport::connect_pair(PeerId(9), PeerId(2));
    let mut server_session = NetworkSession::default();
    let server_config = config("server");
    let duplicate = PlatformIdentity::Iroh {
        node_id: "same-node".into(),
    };
    assert!(server_session.connect_peer(PeerId(7), duplicate.clone()));
    let peer_config = NetworkHandshakeConfig {
        identity: duplicate,
        ..config("duplicate")
    };

    let mut peer_session = NetworkSession::default();
    let (_peer_events, server_events) = pump_handshake(
        &mut peer,
        &mut peer_session,
        &peer_config,
        &mut server,
        &mut server_session,
        &server_config,
    );

    assert!(server_events.is_empty());
    assert!(server_session.peer(PeerId(9)).is_none());
    assert_eq!(
        server_session.peer_for_platform(&peer_config.identity),
        Some(PeerId(7))
    );
}

#[test]
fn malformed_control_payload_rejects_peer() {
    let (mut client, mut server) = MemoryTransport::connect_pair(PeerId(1), PeerId(2));
    let mut server_session = NetworkSession::default();
    let server_config = config("server");

    client.send(
        PeerId(2),
        NetChannel::Control,
        DeliveryMode::Reliable,
        b"not json".to_vec(),
    );
    MemoryTransport::pump_pair(&mut client, &mut server);

    let mut app_events = Vec::new();
    let report = service_control_handshake(
        &mut server,
        &mut server_session,
        &server_config,
        &mut app_events,
    );

    assert_eq!(
        report.rejected_peers,
        [(PeerId(1), HandshakeRejectReason::InvalidControlPayload)]
    );
    assert!(server_session.peer(PeerId(1)).is_none());
    assert!(app_events.is_empty());
}

#[test]
fn disconnect_removes_session_and_forwards_event() {
    let (mut client, mut server) = MemoryTransport::connect_pair(PeerId(1), PeerId(2));
    let mut client_session = NetworkSession::default();
    let mut server_session = NetworkSession::default();
    let client_config = config("client");
    let server_config = config("server");
    pump_handshake(
        &mut client,
        &mut client_session,
        &client_config,
        &mut server,
        &mut server_session,
        &server_config,
    );

    server.disconnect(PeerId(1), DisconnectReason::Timeout);
    let mut app_events = Vec::new();
    let report = service_control_handshake(
        &mut server,
        &mut server_session,
        &server_config,
        &mut app_events,
    );

    assert_eq!(report.disconnected_peers, [PeerId(1)]);
    assert!(server_session.peer(PeerId(1)).is_none());
    assert!(matches!(
        app_events.as_slice(),
        [TransportEvent::Disconnected {
            peer: PeerId(1),
            reason: DisconnectReason::Timeout
        }]
    ));
}
