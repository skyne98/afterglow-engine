use super::*;
use crate::{
    input::PlayerCommand,
    network::{
        NetworkPlayerId,
        handshake::{HandshakeRejectReason, HandshakeReport, service_control_handshake},
        prediction::ClientPredictionBuffer,
        reconciliation::ClientReconciliationQueue,
        replication::WorldSnapshot,
        session::NetworkSession,
    },
};
use std::time::{Duration, Instant};

#[test]
fn iroh_transport_uses_shared_control_handshake() {
    let (mut client, mut server) = connect_pair();
    let client_config = handshake_config("client-node");
    let server_config = handshake_config("server-node");
    let mut client_session = NetworkSession::default();
    let mut server_session = NetworkSession::default();

    client.connect(PeerId(2), server.endpoint_addr());
    let (client_events, server_events) = handshake_pair(
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
fn iroh_handshake_hides_gameplay_packets_until_peer_is_accepted() {
    let (mut client, mut server) = connect_pair();
    wait_for_connection(&mut client, &mut server);
    client.send(
        PeerId(2),
        NetChannel::Commands,
        DeliveryMode::Reliable,
        b"cmd-before-auth".to_vec(),
    );

    let mut server_session = NetworkSession::default();
    let (_events, report) = poll_service_until(
        &mut server,
        &mut server_session,
        &handshake_config("server-node"),
        |events, session, report| {
            events.is_empty()
                && session.peer(PeerId(1)).is_none()
                && report.dropped_unauthorized_packets > 0
        },
    );

    assert_eq!(report.dropped_unauthorized_packets, 1);
    assert!(server_session.peer(PeerId(1)).is_none());
}

#[test]
fn iroh_handshake_forwards_authorized_gameplay_packets() {
    let (mut client, mut server) = connect_pair();
    let client_config = handshake_config("client-node");
    let server_config = handshake_config("server-node");
    let mut client_session = NetworkSession::default();
    let mut server_session = NetworkSession::default();

    client.connect(PeerId(2), server.endpoint_addr());
    handshake_pair(
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

    let (events, report) = poll_service_until(
        &mut server,
        &mut server_session,
        &server_config,
        |events, _session, _report| packet_payloads(events).contains(&b"cmd-after-auth".to_vec()),
    );

    assert_eq!(report.dropped_unauthorized_packets, 0);
    assert_eq!(packet_payloads(&events), [b"cmd-after-auth".to_vec()]);
}

#[test]
fn iroh_protocol_mismatch_rejects_peer_without_session_entry() {
    let (mut client, mut server) = bind_pair(mismatched_protocol(), ProtocolVersion::CURRENT);
    let client_config = handshake_config("client-node");
    let server_config = handshake_config("server-node");
    let mut client_session = NetworkSession::default();
    let mut server_session = NetworkSession::default();
    let mut client_events = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_report = HandshakeReport::default();

    client.connect(PeerId(2), server.endpoint_addr());
    while Instant::now() < deadline {
        service_control_handshake(
            &mut client,
            &mut client_session,
            &client_config,
            &mut client_events,
        );
        last_report = service_control_handshake(
            &mut server,
            &mut server_session,
            &server_config,
            &mut Vec::new(),
        );
        if !last_report.rejected_peers.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(
        last_report.rejected_peers,
        [(
            PeerId(1),
            HandshakeRejectReason::ProtocolMismatch {
                expected: ProtocolVersion::CURRENT,
                got: mismatched_protocol(),
            },
        )]
    );
    assert!(server_session.peer(PeerId(1)).is_none());
}

#[test]
fn iroh_wrong_protocol_gameplay_packet_removes_accepted_peer() {
    let (mut client, mut server) = connect_pair();
    let client_config = handshake_config("client-node");
    let server_config = handshake_config("server-node");
    let mut client_session = NetworkSession::default();
    let mut server_session = NetworkSession::default();

    client.connect(PeerId(2), server.endpoint_addr());
    handshake_pair(
        &mut client,
        &mut client_session,
        &client_config,
        &mut server,
        &mut server_session,
        &server_config,
    );
    client.protocol = mismatched_protocol();
    client.send(
        PeerId(2),
        NetChannel::Commands,
        DeliveryMode::Reliable,
        b"wrong-protocol-cmd".to_vec(),
    );

    let (events, report) = poll_service_until(
        &mut server,
        &mut server_session,
        &server_config,
        |_events, session, report| {
            session.peer(PeerId(1)).is_none() && !report.rejected_peers.is_empty()
        },
    );

    assert!(events.is_empty());
    assert_eq!(
        report.rejected_peers,
        [(
            PeerId(1),
            HandshakeRejectReason::ProtocolMismatch {
                expected: ProtocolVersion::CURRENT,
                got: mismatched_protocol(),
            },
        )]
    );
}

#[test]
fn iroh_disconnect_removes_session_and_forwards_event() {
    let (mut client, mut server) = connect_pair();
    let client_config = handshake_config("client-node");
    let server_config = handshake_config("server-node");
    let mut client_session = NetworkSession::default();
    let mut server_session = NetworkSession::default();

    client.connect(PeerId(2), server.endpoint_addr());
    handshake_pair(
        &mut client,
        &mut client_session,
        &client_config,
        &mut server,
        &mut server_session,
        &server_config,
    );
    client.disconnect(PeerId(2), DisconnectReason::Local);

    let (events, report) = poll_service_until(
        &mut server,
        &mut server_session,
        &server_config,
        |events, session, report| {
            session.peer(PeerId(1)).is_none()
                && report.disconnected_peers == [PeerId(1)]
                && events.iter().any(|event| {
                    matches!(
                        event,
                        TransportEvent::Disconnected {
                            peer: PeerId(1),
                            reason: DisconnectReason::Remote,
                        }
                    )
                })
        },
    );

    assert_eq!(report.disconnected_peers, [PeerId(1)]);
    assert!(server_session.peer(PeerId(1)).is_none());
    assert!(matches!(
        events.as_slice(),
        [TransportEvent::Disconnected {
            peer: PeerId(1),
            reason: DisconnectReason::Remote,
        }]
    ));
}

#[test]
fn iroh_can_drive_snapshot_reconciliation_payloads() {
    let (mut client, mut server) = connect_pair();
    let client_config = handshake_config("client-node");
    let server_config = handshake_config("server-node");
    let mut client_session = NetworkSession::default();
    let mut server_session = NetworkSession::default();

    client.connect(PeerId(2), server.endpoint_addr());
    handshake_pair(
        &mut client,
        &mut client_session,
        &client_config,
        &mut server,
        &mut server_session,
        &server_config,
    );

    let player = NetworkPlayerId(7);
    let mut prediction = ClientPredictionBuffer::default();
    prediction.record(PlayerCommand {
        player,
        tick: 99,
        ..PlayerCommand::default()
    });
    prediction.record(PlayerCommand {
        player,
        tick: 100,
        ..PlayerCommand::default()
    });
    let snapshot = WorldSnapshot {
        tick: 98,
        entities: Vec::new(),
    };
    server.send(
        PeerId(1),
        NetChannel::Snapshots,
        DeliveryMode::UnreliableSequenced,
        serde_json::to_vec(&snapshot).unwrap(),
    );

    let (events, _report) = poll_service_until(
        &mut client,
        &mut client_session,
        &client_config,
        |events, _session, _report| {
            events.iter().any(|event| {
                matches!(event, TransportEvent::Packet(packet) if packet.header.channel == NetChannel::Snapshots)
            })
        },
    );
    let decoded = events
        .iter()
        .find_map(|event| match event {
            TransportEvent::Packet(packet) => {
                serde_json::from_slice::<WorldSnapshot>(&packet.payload).ok()
            }
            _ => None,
        })
        .unwrap();
    let mut reconciliation = ClientReconciliationQueue::default();
    let result = reconciliation.reconcile_snapshot(&mut prediction, player, &decoded);

    assert_eq!(result.authoritative_tick, 98);
    assert_eq!(
        result
            .replay_commands
            .iter()
            .map(|command| command.tick)
            .collect::<Vec<_>>(),
        [99, 100]
    );
}

#[test]
fn iroh_server_accepts_two_clients_and_forwards_both_command_streams() {
    let mut server_config = IrohTransportConfig::local_only();
    server_config.next_inbound_peer = 1;
    let mut server = IrohTransport::bind(PeerId(0), server_config).unwrap();
    let mut alice = IrohTransport::bind(PeerId(1), IrohTransportConfig::local_only()).unwrap();
    let mut bob = IrohTransport::bind(PeerId(2), IrohTransportConfig::local_only()).unwrap();
    let server_config = handshake_config("server-node");
    let alice_config = handshake_config("alice-node");
    let bob_config = handshake_config("bob-node");
    let mut server_session = NetworkSession::default();
    let mut alice_session = NetworkSession::default();
    let mut bob_session = NetworkSession::default();
    let mut server_events = Vec::new();
    let mut alice_events = Vec::new();
    let mut bob_events = Vec::new();

    alice.connect(PeerId(0), server.endpoint_addr());
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && server_session.peer(PeerId(1)).is_none() {
        service_control_handshake(
            &mut alice,
            &mut alice_session,
            &alice_config,
            &mut alice_events,
        );
        service_control_handshake(
            &mut server,
            &mut server_session,
            &server_config,
            &mut server_events,
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    bob.connect(PeerId(0), server.endpoint_addr());
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && server_session.peer(PeerId(2)).is_none() {
        service_control_handshake(
            &mut alice,
            &mut alice_session,
            &alice_config,
            &mut alice_events,
        );
        service_control_handshake(&mut bob, &mut bob_session, &bob_config, &mut bob_events);
        service_control_handshake(
            &mut server,
            &mut server_session,
            &server_config,
            &mut server_events,
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(server_session.peer(PeerId(1)).is_some());
    assert!(server_session.peer(PeerId(2)).is_some());
    alice.send(
        PeerId(0),
        NetChannel::Commands,
        DeliveryMode::Reliable,
        b"alice-cmd".to_vec(),
    );
    bob.send(
        PeerId(0),
        NetChannel::Commands,
        DeliveryMode::Reliable,
        b"bob-cmd".to_vec(),
    );

    let (events, _report) = poll_service_until(
        &mut server,
        &mut server_session,
        &server_config,
        |events, _session, _report| {
            let payloads = packet_payloads(events);
            payloads.contains(&b"alice-cmd".to_vec()) && payloads.contains(&b"bob-cmd".to_vec())
        },
    );

    let payloads = packet_payloads(&events);
    assert!(payloads.contains(&b"alice-cmd".to_vec()));
    assert!(payloads.contains(&b"bob-cmd".to_vec()));
}
