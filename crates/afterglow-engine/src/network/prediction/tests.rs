use super::*;
use crate::input::{InputActionValue, InputAxis, InputAxisValue};

fn command(player: NetworkPlayerId, tick: u32) -> PlayerCommand {
    PlayerCommand {
        player,
        tick,
        axes: vec![InputAxisValue {
            axis: InputAxis::new("move_x"),
            value: tick as f32,
        }],
        actions: vec![InputActionValue::pressed("use")],
        ..Default::default()
    }
}

#[test]
fn records_pending_commands_in_tick_order() {
    let player = NetworkPlayerId(1);
    let mut buffer = ClientPredictionBuffer::default();

    buffer.record(command(player, 3));
    buffer.record(command(player, 1));
    buffer.record(command(player, 2));

    assert_eq!(
        buffer
            .pending(player)
            .map(|command| command.tick)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
}

#[test]
fn command_for_same_player_tick_replaces_previous_value() {
    let player = NetworkPlayerId(1);
    let mut buffer = ClientPredictionBuffer::default();
    let mut replacement = command(player, 2);
    replacement.actions = vec![InputActionValue::pressed("jump")];

    assert!(buffer.record(command(player, 2)).is_none());
    assert_eq!(buffer.record(replacement), Some(command(player, 2)));

    assert_eq!(
        buffer.pending(player).next().unwrap().actions,
        [InputActionValue::pressed("jump")]
    );
}

#[test]
fn acknowledge_prunes_commands_at_or_before_server_tick() {
    let player = NetworkPlayerId(1);
    let mut buffer = ClientPredictionBuffer::default();
    for tick in 1..=5 {
        buffer.record(command(player, tick));
    }

    buffer.acknowledge(player, 3);

    assert_eq!(buffer.acknowledged_tick(player), Some(3));
    assert_eq!(
        buffer
            .pending(player)
            .map(|command| command.tick)
            .collect::<Vec<_>>(),
        [4, 5]
    );
}

#[test]
fn older_acknowledgements_do_not_move_tick_backwards() {
    let player = NetworkPlayerId(1);
    let mut buffer = ClientPredictionBuffer::default();

    buffer.acknowledge(player, 10);
    buffer.acknowledge(player, 4);

    assert_eq!(buffer.acknowledged_tick(player), Some(10));
}

#[test]
fn replay_after_returns_unacknowledged_commands() {
    let player = NetworkPlayerId(1);
    let mut buffer = ClientPredictionBuffer::default();
    for tick in 1..=4 {
        buffer.record(command(player, tick));
    }

    let replay = buffer.replay_after(player, 2);

    assert_eq!(replay.player, player);
    assert_eq!(replay.authoritative_tick, 2);
    assert_eq!(
        replay
            .commands
            .iter()
            .map(|command| command.tick)
            .collect::<Vec<_>>(),
        [3, 4]
    );
}

#[test]
fn players_have_independent_prediction_history() {
    let alice = NetworkPlayerId(1);
    let bob = NetworkPlayerId(2);
    let mut buffer = ClientPredictionBuffer::default();
    buffer.record(command(alice, 1));
    buffer.record(command(bob, 1));

    buffer.acknowledge(alice, 1);

    assert_eq!(buffer.pending_len(alice), 0);
    assert_eq!(buffer.pending_len(bob), 1);
}

#[test]
fn clear_player_removes_pending_and_ack_state() {
    let player = NetworkPlayerId(1);
    let mut buffer = ClientPredictionBuffer::default();
    buffer.record(command(player, 1));
    buffer.acknowledge(player, 1);

    buffer.clear_player(player);

    assert_eq!(buffer.pending_len(player), 0);
    assert_eq!(buffer.acknowledged_tick(player), None);
}
