use super::*;
use crate::network::{
    handshake::{NetworkBackendKind, NetworkHandshakeConfig, service_control_handshake},
    session::{NetworkSession, PlatformIdentity},
};
use std::time::{Duration, Instant};

fn poll_until(
    transport: &mut IrohTransport,
    matches: impl Fn(&[TransportEvent]) -> bool,
) -> Vec<TransportEvent> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut events = Vec::new();
    while Instant::now() < deadline {
        transport.poll_events(&mut events);
        if matches(&events) {
            return events;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    events
}

fn poll_for(transport: &mut IrohTransport, duration: Duration) -> Vec<TransportEvent> {
    let deadline = Instant::now() + duration;
    let mut events = Vec::new();
    while Instant::now() < deadline {
        transport.poll_events(&mut events);
        std::thread::sleep(Duration::from_millis(10));
    }
    events
}

fn connect_pair() -> (IrohTransport, IrohTransport) {
    let client = IrohTransport::bind(PeerId(1), IrohTransportConfig::local_only()).unwrap();
    let mut server_config = IrohTransportConfig::local_only();
    server_config.next_inbound_peer = 1;
    let server = IrohTransport::bind(PeerId(2), server_config).unwrap();
    (client, server)
}

fn wait_for_connection(client: &mut IrohTransport, server: &mut IrohTransport) {
    client.connect(PeerId(2), server.endpoint_addr());
    let client_events = poll_until(client, |events| {
        events.contains(&TransportEvent::Connected(PeerId(2)))
    });
    let server_events = poll_until(server, |events| {
        events.contains(&TransportEvent::Connected(PeerId(1)))
    });
    assert!(client_events.contains(&TransportEvent::Connected(PeerId(2))));
    assert!(server_events.contains(&TransportEvent::Connected(PeerId(1))));
}

fn handshake_config(label: &str) -> NetworkHandshakeConfig {
    NetworkHandshakeConfig {
        protocol: ProtocolVersion::CURRENT,
        build_hash: "iroh-test-build".into(),
        content_hash: "iroh-test-content".into(),
        backend: NetworkBackendKind::Iroh,
        identity: PlatformIdentity::Iroh {
            node_id: label.into(),
        },
    }
}

#[test]
fn iroh_transport_connects_two_local_endpoints_and_delivers_reliable_packet() {
    let (mut client, mut server) = connect_pair();
    wait_for_connection(&mut client, &mut server);

    client.send(
        PeerId(2),
        NetChannel::Control,
        DeliveryMode::Reliable,
        b"hello".to_vec(),
    );
    let events = poll_until(&mut server, |events| {
        events.iter().any(
            |event| matches!(event, TransportEvent::Packet(packet) if packet.payload == b"hello"),
        )
    });

    assert!(events.iter().any(|event| {
        matches!(
            event,
            TransportEvent::Packet(packet)
                if packet.from == PeerId(1)
                    && packet.to == PeerId(2)
                    && packet.header.channel == NetChannel::Control
                    && packet.header.delivery == DeliveryMode::Reliable
                    && packet.payload == b"hello"
        )
    }));
}

#[test]
fn iroh_transport_preserves_reliable_send_order() {
    let (mut client, mut server) = connect_pair();
    wait_for_connection(&mut client, &mut server);

    for payload in ["first", "second", "third"] {
        client.send(
            PeerId(2),
            NetChannel::Control,
            DeliveryMode::Reliable,
            payload.as_bytes().to_vec(),
        );
    }
    let events = poll_until(&mut server, |events| {
        events
            .iter()
            .filter(|event| matches!(event, TransportEvent::Packet(_)))
            .count()
            >= 3
    });
    let payloads = events
        .into_iter()
        .filter_map(|event| match event {
            TransportEvent::Packet(packet) => Some(packet.payload),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        payloads,
        [b"first".to_vec(), b"second".to_vec(), b"third".to_vec()]
    );
}

#[test]
fn iroh_transport_delivers_unreliable_packet() {
    let (mut client, mut server) = connect_pair();
    wait_for_connection(&mut client, &mut server);

    client.send(
        PeerId(2),
        NetChannel::Commands,
        DeliveryMode::Unreliable,
        b"datagram".to_vec(),
    );
    let events = poll_until(&mut server, |events| {
        events.iter().any(|event| {
            matches!(event, TransportEvent::Packet(packet) if packet.payload == b"datagram")
        })
    });

    assert!(events.iter().any(|event| {
        matches!(
            event,
            TransportEvent::Packet(packet)
                if packet.header.channel == NetChannel::Commands
                    && packet.header.delivery == DeliveryMode::Unreliable
                    && packet.payload == b"datagram"
        )
    }));
}

#[test]
fn iroh_transport_drops_stale_unreliable_sequenced_packets() {
    let (mut client, mut server) = connect_pair();
    wait_for_connection(&mut client, &mut server);
    server
        .delivered_sequences
        .insert((PeerId(1), NetChannel::Commands), 10);

    client.send(
        PeerId(2),
        NetChannel::Commands,
        DeliveryMode::UnreliableSequenced,
        b"stale".to_vec(),
    );
    client.send(
        PeerId(2),
        NetChannel::Bulk,
        DeliveryMode::Reliable,
        b"marker".to_vec(),
    );

    let events = poll_until(&mut server, |events| {
        events.iter().any(
            |event| matches!(event, TransportEvent::Packet(packet) if packet.payload == b"marker"),
        )
    });

    assert!(events.iter().any(|event| {
        matches!(event, TransportEvent::Packet(packet) if packet.payload == b"marker")
    }));
    assert!(!events.iter().any(|event| {
        matches!(event, TransportEvent::Packet(packet) if packet.payload == b"stale")
    }));
    let late_events = poll_for(&mut server, Duration::from_millis(250));
    assert!(!late_events.iter().any(|event| {
        matches!(event, TransportEvent::Packet(packet) if packet.payload == b"stale")
    }));
}

#[test]
fn iroh_transport_reports_remote_disconnects() {
    let (mut client, mut server) = connect_pair();
    wait_for_connection(&mut client, &mut server);

    server.disconnect(PeerId(1), DisconnectReason::Local);
    let events = poll_until(&mut client, |events| {
        events.iter().any(|event| {
            matches!(
                event,
                TransportEvent::Disconnected {
                    peer: PeerId(2),
                    reason: DisconnectReason::Remote,
                }
            )
        })
    });

    assert!(events.iter().any(|event| {
        matches!(
            event,
            TransportEvent::Disconnected {
                peer: PeerId(2),
                reason: DisconnectReason::Remote,
            }
        )
    }));
}

#[test]
fn iroh_transport_uses_shared_control_handshake() {
    let (mut client, mut server) = connect_pair();
    let client_handshake = handshake_config("client-node");
    let server_handshake = handshake_config("server-node");
    let mut client_session = NetworkSession::default();
    let mut server_session = NetworkSession::default();

    client.connect(PeerId(2), server.endpoint_addr());
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut client_events = Vec::new();
    let mut server_events = Vec::new();
    while Instant::now() < deadline {
        service_control_handshake(
            &mut client,
            &mut client_session,
            &client_handshake,
            &mut client_events,
        );
        service_control_handshake(
            &mut server,
            &mut server_session,
            &server_handshake,
            &mut server_events,
        );
        if client_session.peer(PeerId(2)).is_some() && server_session.peer(PeerId(1)).is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(
        client_session.peer(PeerId(2)).unwrap().platform,
        server_handshake.identity
    );
    assert_eq!(
        server_session.peer(PeerId(1)).unwrap().platform,
        client_handshake.identity
    );
    assert_eq!(client_events, [TransportEvent::Connected(PeerId(2))]);
    assert_eq!(server_events, [TransportEvent::Connected(PeerId(1))]);
}
