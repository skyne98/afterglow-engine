use super::*;

#[test]
fn steam_lobby_metadata_exports_stable_keys() {
    let metadata = SteamLobbyMetadata {
        protocol: ProtocolVersion::CURRENT,
        build_hash: "build".into(),
        content_hash: "content".into(),
        world_id: "cellar".into(),
        host_steam_id: 42,
        host_virtual_port: 7,
    };

    assert_eq!(
        metadata.entries(),
        [
            ("protocol", "0.2.0".into()),
            ("build_hash", "build".into()),
            ("content_hash", "content".into()),
            ("world_id", "cellar".into()),
            ("host_steam_id", "42".into()),
            ("host_virtual_port", "7".into()),
        ]
    );
}

#[test]
fn steam_delivery_modes_map_to_steam_send_flags() {
    assert_eq!(send_flags(DeliveryMode::Reliable), SendFlags::RELIABLE);
    assert_eq!(send_flags(DeliveryMode::Unreliable), SendFlags::UNRELIABLE);
    assert_eq!(
        send_flags(DeliveryMode::UnreliableSequenced),
        SendFlags::UNRELIABLE
    );
}

#[test]
fn steam_wire_packets_roundtrip_without_steam_client() {
    let packet = NetworkPacket {
        from: PeerId(1),
        to: PeerId(2),
        header: PacketHeader {
            protocol: ProtocolVersion::CURRENT,
            channel: NetChannel::Snapshots,
            delivery: DeliveryMode::UnreliableSequenced,
            sequence: 9,
        },
        payload: b"snapshot".to_vec(),
    };

    let encoded = encode_transport_packet(&packet).unwrap();
    let decoded = decode_transport_packet(PeerId(1), PeerId(2), &encoded).unwrap();

    assert_eq!(decoded, packet);
}
