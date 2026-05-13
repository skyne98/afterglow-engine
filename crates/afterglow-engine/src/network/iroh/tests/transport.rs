use super::*;

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

    assert_eq!(
        packet_payloads(&events),
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

    assert!(packet_payloads(&events).contains(&b"marker".to_vec()));
    assert!(!packet_payloads(&events).contains(&b"stale".to_vec()));
    let late_events = poll_for(&mut server, Duration::from_millis(250));
    assert!(!packet_payloads(&late_events).contains(&b"stale".to_vec()));
}

#[test]
fn iroh_transport_resets_unreliable_sequence_state_on_reconnect() {
    let (mut client, mut server) = connect_pair();
    wait_for_connection(&mut client, &mut server);

    client
        .delivered_sequences
        .insert((PeerId(2), NetChannel::Commands), 10);
    server.disconnect(PeerId(1), DisconnectReason::Local);
    let _ = poll_until(&mut client, |events| {
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
    client.connect(PeerId(2), server.endpoint_addr());
    let client_events = poll_until(&mut client, |events| {
        events.contains(&TransportEvent::Connected(PeerId(2)))
    });
    let server_events = poll_until(&mut server, |events| {
        events.contains(&TransportEvent::Connected(PeerId(3)))
    });

    assert!(client_events.contains(&TransportEvent::Connected(PeerId(2))));
    assert!(server_events.contains(&TransportEvent::Connected(PeerId(3))));
    server.send(
        PeerId(3),
        NetChannel::Commands,
        DeliveryMode::UnreliableSequenced,
        b"fresh-after-reconnect".to_vec(),
    );
    let events = poll_until(&mut client, |events| {
        packet_payloads(events).contains(&b"fresh-after-reconnect".to_vec())
    });

    assert!(packet_payloads(&events).contains(&b"fresh-after-reconnect".to_vec()));
}

#[test]
fn iroh_transport_ignores_packets_from_disconnected_peers() {
    let (mut client, mut server) = connect_pair();
    wait_for_connection(&mut client, &mut server);

    server.disconnect(PeerId(1), DisconnectReason::Local);
    let server_events = poll_until(&mut server, |events| {
        events.iter().any(|event| {
            matches!(
                event,
                TransportEvent::Disconnected {
                    peer: PeerId(1),
                    reason: DisconnectReason::Local,
                }
            )
        })
    });
    assert!(server_events.iter().any(|event| {
        matches!(
            event,
            TransportEvent::Disconnected {
                peer: PeerId(1),
                reason: DisconnectReason::Local,
            }
        )
    }));

    client.send(
        PeerId(2),
        NetChannel::Commands,
        DeliveryMode::Reliable,
        b"after-disconnect".to_vec(),
    );
    let late_events = poll_for(&mut server, Duration::from_millis(250));

    assert!(!packet_payloads(&late_events).contains(&b"after-disconnect".to_vec()));
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
