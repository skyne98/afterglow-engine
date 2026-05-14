use super::*;
use crate::input::{InputActionValue, PlayerCommand};

fn command(player: NetworkPlayerId, tick: u32) -> PlayerCommand {
    PlayerCommand {
        player,
        tick,
        actions: vec![InputActionValue::pressed("use")],
        ..Default::default()
    }
}

#[test]
fn correction_prunes_acknowledged_commands_and_returns_replay_tail() {
    let player = NetworkPlayerId(1);
    let mut prediction = ClientPredictionBuffer::default();
    for tick in 1..=5 {
        prediction.record(command(player, tick));
    }

    let result = reconcile(
        &mut prediction,
        AuthoritativeCorrection {
            player,
            tick: 3,
            source: AuthoritativeUpdateSource::Correction,
        },
    );

    assert_eq!(result.player, player);
    assert_eq!(result.authoritative_tick, 3);
    assert_eq!(result.source, AuthoritativeUpdateSource::Correction);
    assert_eq!(
        result
            .replay_commands
            .iter()
            .map(|command| command.tick)
            .collect::<Vec<_>>(),
        [4, 5]
    );
    assert_eq!(prediction.acknowledged_tick(player), Some(3));
}

#[test]
fn snapshot_reconciliation_uses_snapshot_tick() {
    let player = NetworkPlayerId(1);
    let mut prediction = ClientPredictionBuffer::default();
    let mut queue = ClientReconciliationQueue::default();
    for tick in 10..=12 {
        prediction.record(command(player, tick));
    }
    let snapshot = WorldSnapshot {
        tick: 10,
        entities: Vec::new(),
    };

    let result = queue.reconcile_snapshot(&mut prediction, player, &snapshot);

    assert_eq!(result.source, AuthoritativeUpdateSource::Snapshot);
    assert_eq!(
        result
            .replay_commands
            .iter()
            .map(|command| command.tick)
            .collect::<Vec<_>>(),
        [11, 12]
    );
    assert_eq!(queue.results().len(), 1);
}

#[test]
fn delta_reconciliation_uses_delta_to_tick() {
    let player = NetworkPlayerId(1);
    let mut prediction = ClientPredictionBuffer::default();
    let mut queue = ClientReconciliationQueue::default();
    for tick in 1..=4 {
        prediction.record(command(player, tick));
    }
    let delta = WorldDelta {
        from_tick: 1,
        to_tick: 2,
        changes: Vec::new(),
        removed: Vec::new(),
    };

    let result = queue.reconcile_delta(&mut prediction, player, &delta);

    assert_eq!(result.source, AuthoritativeUpdateSource::Delta);
    assert_eq!(
        result
            .replay_commands
            .iter()
            .map(|command| command.tick)
            .collect::<Vec<_>>(),
        [3, 4]
    );
}

#[test]
fn queue_clear_removes_frame_results_without_touching_prediction_history() {
    let player = NetworkPlayerId(1);
    let mut prediction = ClientPredictionBuffer::default();
    let mut queue = ClientReconciliationQueue::default();
    prediction.record(command(player, 2));

    queue.reconcile(
        &mut prediction,
        AuthoritativeCorrection {
            player,
            tick: 1,
            source: AuthoritativeUpdateSource::Correction,
        },
    );
    queue.clear();

    assert!(queue.results().is_empty());
    assert_eq!(prediction.pending_len(player), 1);
}

#[test]
fn players_reconcile_independently() {
    let alice = NetworkPlayerId(1);
    let bob = NetworkPlayerId(2);
    let mut prediction = ClientPredictionBuffer::default();
    prediction.record(command(alice, 1));
    prediction.record(command(alice, 2));
    prediction.record(command(bob, 1));

    let result = reconcile(
        &mut prediction,
        AuthoritativeCorrection {
            player: alice,
            tick: 1,
            source: AuthoritativeUpdateSource::Correction,
        },
    );

    assert_eq!(result.replay_commands.len(), 1);
    assert_eq!(prediction.pending_len(alice), 1);
    assert_eq!(prediction.pending_len(bob), 1);
}
