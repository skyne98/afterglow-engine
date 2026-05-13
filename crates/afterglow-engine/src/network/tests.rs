use super::*;
use crate::testing::unit_app;

#[test]
fn network_plugin_registers_protocol_resource() {
    let mut app = unit_app();
    app.add_plugins(AfterglowNetworkPlugin);

    assert_eq!(
        app.world().resource::<NetworkProtocol>().version,
        ProtocolVersion::CURRENT
    );
    app.world().resource::<baseline::ReconnectBaselineStore>();
    app.world()
        .resource::<rollback::DeterministicRollbackBuffer>();
    app.world().resource::<authority::ServerCommandBuffer>();
    app.world().resource::<prediction::ClientPredictionBuffer>();
    app.world()
        .resource::<reconciliation::ClientReconciliationQueue>();
    app.world()
        .resource::<interpolation::RemoteInterpolationBuffer>();
    app.world().resource::<interest::InterestMap>();
    app.world()
        .resource::<replication::RollbackReplicationClock>();
    app.world().resource::<session::NetworkSession>();
}

#[test]
fn memory_transport_connects_and_delivers_packets() {
    let (mut a, mut b) = MemoryTransport::connect_pair(PeerId(1), PeerId(2));
    let mut events = Vec::new();
    a.poll_events(&mut events);
    assert_eq!(events, [TransportEvent::Connected(PeerId(2))]);

    a.send(
        PeerId(2),
        NetChannel::Commands,
        DeliveryMode::UnreliableSequenced,
        b"cmd".to_vec(),
    );
    MemoryTransport::pump_pair(&mut a, &mut b);

    events.clear();
    b.poll_events(&mut events);
    assert_eq!(
        events,
        [
            TransportEvent::Connected(PeerId(1)),
            TransportEvent::Packet(NetworkPacket {
                from: PeerId(1),
                to: PeerId(2),
                header: PacketHeader {
                    protocol: ProtocolVersion::CURRENT,
                    channel: NetChannel::Commands,
                    delivery: DeliveryMode::UnreliableSequenced,
                    sequence: 0,
                },
                payload: b"cmd".to_vec(),
            })
        ]
    );
}

#[test]
fn memory_transport_applies_loss_duplication_and_reorder() {
    let mut a = MemoryTransport::new(PeerId(1));
    let mut b = MemoryTransport::new(PeerId(2)).with_faults(FaultConfig {
        drop_every: Some(2),
        duplicate_every: Some(3),
        delay_ticks: 0,
        reverse_delivery: true,
    });
    a.connected.insert(PeerId(2));
    b.connected.insert(PeerId(1));

    for byte in [0, 1, 2] {
        a.send(
            PeerId(2),
            NetChannel::Events,
            DeliveryMode::Reliable,
            vec![byte],
        );
    }
    MemoryTransport::pump_pair(&mut a, &mut b);

    let mut events = Vec::new();
    b.poll_events(&mut events);
    let payloads = events
        .into_iter()
        .filter_map(|event| match event {
            TransportEvent::Packet(packet) => Some(packet.payload),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(payloads, [vec![2], vec![2], vec![0]]);
}

#[test]
fn memory_transport_drops_stale_unreliable_sequenced_packets() {
    let mut a = MemoryTransport::new(PeerId(1));
    let mut b = MemoryTransport::new(PeerId(2)).with_faults(FaultConfig {
        duplicate_every: Some(2),
        reverse_delivery: true,
        ..default()
    });
    a.connected.insert(PeerId(2));
    b.connected.insert(PeerId(1));

    for byte in [0, 1, 2] {
        a.send(
            PeerId(2),
            NetChannel::Commands,
            DeliveryMode::UnreliableSequenced,
            vec![byte],
        );
    }
    MemoryTransport::pump_pair(&mut a, &mut b);

    let mut events = Vec::new();
    b.poll_events(&mut events);
    let payloads = events
        .into_iter()
        .filter_map(|event| match event {
            TransportEvent::Packet(packet) => Some(packet.payload),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(payloads, [vec![2]]);
}

#[test]
fn memory_transport_resets_unreliable_sequence_state_on_reconnect() {
    let (mut a, mut b) = MemoryTransport::connect_pair(PeerId(1), PeerId(2));
    a.send(
        PeerId(2),
        NetChannel::Commands,
        DeliveryMode::UnreliableSequenced,
        vec![1],
    );
    MemoryTransport::pump_pair(&mut a, &mut b);
    let mut events = Vec::new();
    b.poll_events(&mut events);
    assert!(events.iter().any(|event| matches!(
        event,
        TransportEvent::Packet(packet) if packet.payload == vec![1]
    )));

    b.disconnect(PeerId(1), DisconnectReason::Remote);
    b.poll_events(&mut Vec::new());
    b.connect_peer(PeerId(1));
    a = MemoryTransport::new(PeerId(1));
    a.connect_peer(PeerId(2));
    a.send(
        PeerId(2),
        NetChannel::Commands,
        DeliveryMode::UnreliableSequenced,
        vec![2],
    );

    b.receive_packets(a.collect_outgoing());
    events.clear();
    b.poll_events(&mut events);

    assert!(events.iter().any(|event| matches!(
        event,
        TransportEvent::Packet(packet) if packet.payload == vec![2]
    )));
}

#[test]
fn memory_transport_ignores_packets_from_disconnected_peers() {
    let (mut a, mut b) = MemoryTransport::connect_pair(PeerId(1), PeerId(2));
    b.disconnect(PeerId(1), DisconnectReason::Local);
    b.poll_events(&mut Vec::new());

    a.send(
        PeerId(2),
        NetChannel::Commands,
        DeliveryMode::Reliable,
        vec![1],
    );
    b.receive_packets(a.collect_outgoing());

    let mut events = Vec::new();
    b.poll_events(&mut events);

    assert!(events.is_empty());
}

#[test]
fn memory_transport_can_delay_packets_by_ticks() {
    let (mut a, mut b) = MemoryTransport::connect_pair(PeerId(1), PeerId(2));
    b = b.with_faults(FaultConfig {
        delay_ticks: 2,
        ..default()
    });
    a.send(
        PeerId(2),
        NetChannel::Snapshots,
        DeliveryMode::Unreliable,
        b"state".to_vec(),
    );

    let mut events = Vec::new();
    MemoryTransport::pump_pair(&mut a, &mut b);
    b.poll_events(&mut events);
    assert_eq!(events, [TransportEvent::Connected(PeerId(1))]);

    events.clear();
    MemoryTransport::pump_pair(&mut a, &mut b);
    b.poll_events(&mut events);
    assert!(matches!(events.as_slice(), [TransportEvent::Packet(_)]));
}
