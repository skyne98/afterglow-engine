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
