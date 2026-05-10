use super::*;

#[test]
fn player_commands_roundtrip_through_wire_envelope() {
    let command = PlayerCommand {
        player: NetworkPlayerId(7),
        tick: 42,
        axes: vec![InputAxisValue {
            axis: InputAxis::new("move.x"),
            value: 0.5,
        }],
        actions: vec![InputAction::new("use")],
        pointers: vec![PointerInput {
            device: PointerDevice::Pen,
            id: 9,
            position: Vec2::new(10.0, 20.0),
            delta: Vec2::new(1.0, -1.0),
            pressure: Some(0.75),
            tilt: Some(Vec2::new(0.2, -0.4)),
            twist: Some(0.1),
            primary: true,
        }],
    };

    let bytes = encode_player_commands(std::slice::from_ref(&command)).unwrap();
    let decoded = decode_player_commands(&bytes).unwrap();

    assert_eq!(decoded, [command]);
}

#[test]
fn malformed_command_payload_is_rejected() {
    let err = decode_player_commands(b"not json").unwrap_err();

    assert!(matches!(err, CommandDecodeError::InvalidJson(_)));
}

#[test]
fn protocol_mismatch_is_rejected() {
    let bytes = serde_json::to_vec(&CommandEnvelope {
        protocol: WireProtocolVersion {
            major: 999,
            minor: 0,
            patch: 0,
        },
        commands: Vec::new(),
    })
    .unwrap();

    let err = decode_player_commands(&bytes).unwrap_err();

    assert_eq!(
        err,
        CommandDecodeError::ProtocolMismatch {
            expected: ProtocolVersion::CURRENT,
            got: ProtocolVersion {
                major: 999,
                minor: 0,
                patch: 0,
            },
        }
    );
}

#[test]
fn multiple_local_commands_preserve_player_and_tick_order() {
    let commands = vec![
        PlayerCommand {
            player: NetworkPlayerId(1),
            tick: 5,
            ..default()
        },
        PlayerCommand {
            player: NetworkPlayerId(2),
            tick: 5,
            ..default()
        },
    ];

    let bytes = encode_player_commands(&commands).unwrap();
    let decoded = decode_player_commands(&bytes).unwrap();

    assert_eq!(
        decoded
            .iter()
            .map(|command| (command.player, command.tick))
            .collect::<Vec<_>>(),
        [(NetworkPlayerId(1), 5), (NetworkPlayerId(2), 5)]
    );
}
