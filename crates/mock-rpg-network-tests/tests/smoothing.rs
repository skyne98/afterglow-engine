use afterglow_engine::{
    core::identity::StableEntityId,
    input::{InputAction, InputAxis, InputAxisValue, PlayerCommand},
    network::{
        NetworkPlayerId,
        interpolation::{RemoteEntitySample, RemoteInterpolationBuffer, SmoothingMode},
        prediction::ClientPredictionBuffer,
        reconciliation::ClientReconciliationQueue,
        replication::WorldSnapshot,
    },
};
use mock_rpg_network_tests::{Player, Vec3i};

fn player_id(player: Player) -> NetworkPlayerId {
    NetworkPlayerId(player.0)
}

fn avatar_id(player: Player) -> StableEntityId {
    StableEntityId::from_raw(10_000 + player.0 as u128)
}

fn move_command(player: Player, tick: u32, x: f32) -> PlayerCommand {
    PlayerCommand {
        player: player_id(player),
        tick,
        axes: vec![InputAxisValue {
            axis: InputAxis::new("move_x"),
            value: x,
        }],
        ..Default::default()
    }
}

fn use_command(player: Player, tick: u32) -> PlayerCommand {
    PlayerCommand {
        player: player_id(player),
        tick,
        actions: vec![InputAction::new("use")],
        ..Default::default()
    }
}

fn apply_mock_move(position: &mut Vec3i, command: &PlayerCommand) {
    let x = command
        .axes
        .iter()
        .find(|axis| axis.axis == InputAxis::new("move_x"))
        .map_or(0, |axis| axis.value.round() as i32);
    position.x += x;
}

fn remote_sample(position: Vec3i) -> RemoteEntitySample {
    RemoteEntitySample::default()
        .with_field("pos_x", position.x as f32)
        .with_field("pos_y", position.y as f32)
        .with_field("pos_z", position.z as f32)
}

#[test]
fn predicted_movement_keeps_commands_until_authoritative_tick_acknowledges_them() {
    let alice = Player(1);
    let mut prediction = ClientPredictionBuffer::default();
    let mut reconciliation = ClientReconciliationQueue::default();
    let tick_99 = move_command(alice, 99, 1.0);
    let tick_100 = move_command(alice, 100, 1.0);
    prediction.record(tick_99.clone());
    prediction.record(tick_100.clone());

    let mut predicted_position = Vec3i::ZERO;
    apply_mock_move(&mut predicted_position, &tick_99);
    apply_mock_move(&mut predicted_position, &tick_100);
    assert_eq!(predicted_position, Vec3i::new(2, 0, 0));

    let authoritative_98 = WorldSnapshot {
        tick: 98,
        entities: Vec::new(),
    };
    let replay_98 =
        reconciliation.reconcile_snapshot(&mut prediction, player_id(alice), &authoritative_98);
    assert_eq!(
        replay_98
            .replay_commands
            .iter()
            .map(|command| command.tick)
            .collect::<Vec<_>>(),
        [99, 100]
    );
    assert_eq!(prediction.pending_len(player_id(alice)), 2);

    let authoritative_99 = WorldSnapshot {
        tick: 99,
        entities: Vec::new(),
    };
    let replay_99 =
        reconciliation.reconcile_snapshot(&mut prediction, player_id(alice), &authoritative_99);
    assert_eq!(
        replay_99
            .replay_commands
            .iter()
            .map(|command| command.tick)
            .collect::<Vec<_>>(),
        [100]
    );
    assert_eq!(prediction.pending_len(player_id(alice)), 1);

    let authoritative_100 = WorldSnapshot {
        tick: 100,
        entities: Vec::new(),
    };
    let replay_100 =
        reconciliation.reconcile_snapshot(&mut prediction, player_id(alice), &authoritative_100);
    assert!(replay_100.replay_commands.is_empty());
    assert_eq!(prediction.pending_len(player_id(alice)), 0);
}

#[test]
fn predicted_interaction_feedback_is_replayed_until_server_confirms_or_rejects_tick() {
    let alice = Player(1);
    let mut prediction = ClientPredictionBuffer::default();
    let mut reconciliation = ClientReconciliationQueue::default();
    prediction.record(use_command(alice, 200));

    let authoritative_198 = WorldSnapshot {
        tick: 198,
        entities: Vec::new(),
    };
    let replay =
        reconciliation.reconcile_snapshot(&mut prediction, player_id(alice), &authoritative_198);
    assert_eq!(replay.replay_commands[0].actions, [InputAction::new("use")]);
    assert_eq!(prediction.pending_len(player_id(alice)), 1);

    let authoritative_200 = WorldSnapshot {
        tick: 200,
        entities: Vec::new(),
    };
    let replay =
        reconciliation.reconcile_snapshot(&mut prediction, player_id(alice), &authoritative_200);
    assert!(replay.replay_commands.is_empty());
    assert_eq!(prediction.pending_len(player_id(alice)), 0);
}

#[test]
fn remote_player_positions_interpolate_between_authoritative_snapshots() {
    let bob = avatar_id(Player(2));
    let mut interpolation = RemoteInterpolationBuffer::default().with_timing(2, 2);
    interpolation.record(bob, 100, remote_sample(Vec3i::new(0, 0, 0)));
    interpolation.record(bob, 102, remote_sample(Vec3i::new(10, 0, 0)));

    let smoothed = interpolation.sample_at(bob, 101.0).unwrap();

    assert_eq!(smoothed.mode, SmoothingMode::Interpolated);
    assert_eq!(smoothed.fields["pos_x"], 5.0);
}

#[test]
fn remote_player_positions_extrapolate_briefly_when_a_packet_is_late() {
    let bob = avatar_id(Player(2));
    let mut interpolation = RemoteInterpolationBuffer::default().with_timing(2, 1);
    interpolation.record(bob, 100, remote_sample(Vec3i::new(0, 0, 0)));
    interpolation.record(bob, 102, remote_sample(Vec3i::new(10, 0, 0)));

    let smoothed = interpolation.sample_at(bob, 103.0).unwrap();
    assert_eq!(smoothed.mode, SmoothingMode::Extrapolated);
    assert_eq!(smoothed.fields["pos_x"], 15.0);

    assert!(interpolation.sample_at(bob, 104.0).is_none());
}
