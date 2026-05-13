use super::*;

#[test]
fn accepted_peer_wrong_protocol_gameplay_packet_is_rejected() {
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
    client = client.with_protocol(ProtocolVersion {
        major: 4,
        minor: 0,
        patch: 0,
    });

    client.send(
        PeerId(2),
        NetChannel::Commands,
        DeliveryMode::Reliable,
        b"wrong-protocol-cmd".to_vec(),
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
                    major: 4,
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
            major: 4,
            minor: 0,
            patch: 0,
        }
    )));
}

#[test]
fn repeated_bad_packets_after_rejection_are_dropped_in_same_service_pass() {
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
    client = client.with_protocol(ProtocolVersion {
        major: 5,
        minor: 0,
        patch: 0,
    });

    for payload in [b"bad-a".to_vec(), b"bad-b".to_vec()] {
        client.send(
            PeerId(2),
            NetChannel::Commands,
            DeliveryMode::Reliable,
            payload,
        );
    }
    MemoryTransport::pump_pair(&mut client, &mut server);

    let mut app_events = Vec::new();
    let report = service_control_handshake(
        &mut server,
        &mut server_session,
        &server_config,
        &mut app_events,
    );

    assert_eq!(report.rejected_peers.len(), 1);
    assert_eq!(report.dropped_unauthorized_packets, 1);
    assert!(server_session.peer(PeerId(1)).is_none());
    assert!(app_events.is_empty());
}

#[test]
fn accepted_control_payload_with_wrong_protocol_is_rejected() {
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
        NetChannel::Control,
        DeliveryMode::Reliable,
        encode_control_message(&ControlMessage::Accepted(ControlAccepted {
            protocol: ProtocolVersion {
                major: 6,
                minor: 0,
                patch: 0,
            },
        })),
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
                    major: 6,
                    minor: 0,
                    patch: 0,
                },
            },
        )]
    );
    assert!(server_session.peer(PeerId(1)).is_none());
    assert!(app_events.is_empty());
}

#[test]
fn accepted_control_without_prior_hello_is_rejected() {
    let (mut client, mut server) = MemoryTransport::connect_pair(PeerId(1), PeerId(2));
    let mut server_session = NetworkSession::default();
    let server_config = config("server");

    client.send(
        PeerId(2),
        NetChannel::Control,
        DeliveryMode::Reliable,
        encode_control_message(&ControlMessage::Accepted(ControlAccepted {
            protocol: ProtocolVersion::CURRENT,
        })),
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
fn repeated_matching_hello_is_idempotent() {
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
        NetChannel::Control,
        DeliveryMode::Reliable,
        encode_control_message(&client_config.hello()),
    );
    MemoryTransport::pump_pair(&mut client, &mut server);

    let mut app_events = Vec::new();
    let report = service_control_handshake(
        &mut server,
        &mut server_session,
        &server_config,
        &mut app_events,
    );

    assert!(report.accepted_peers.is_empty());
    assert!(report.rejected_peers.is_empty());
    assert!(app_events.is_empty());
    assert_eq!(
        server_session.peer(PeerId(1)).unwrap().platform,
        client_config.identity
    );
}

#[test]
fn repeated_hello_with_changed_identity_is_rejected() {
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
    let changed_config = NetworkHandshakeConfig {
        identity: PlatformIdentity::Anonymous {
            label: "changed-client".into(),
        },
        ..client_config
    };

    client.send(
        PeerId(2),
        NetChannel::Control,
        DeliveryMode::Reliable,
        encode_control_message(&changed_config.hello()),
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
        [(PeerId(1), HandshakeRejectReason::PeerIdentityChanged)]
    );
    assert!(server_session.peer(PeerId(1)).is_none());
    assert!(app_events.is_empty());
}

#[test]
fn rejected_control_message_removes_session_before_later_same_batch_gameplay() {
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
        NetChannel::Control,
        DeliveryMode::Reliable,
        encode_control_message(&ControlMessage::Rejected(ControlReject {
            reason: HandshakeRejectReason::BuildMismatch,
        })),
    );
    client.send(
        PeerId(2),
        NetChannel::Commands,
        DeliveryMode::Reliable,
        b"must-not-forward".to_vec(),
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
        [(PeerId(1), HandshakeRejectReason::BuildMismatch)]
    );
    assert_eq!(report.dropped_unauthorized_packets, 1);
    assert!(server_session.peer(PeerId(1)).is_none());
    assert!(app_events.is_empty());
}
