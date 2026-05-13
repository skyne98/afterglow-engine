use super::*;
use crate::core::identity::StableEntityId;

fn command(tick: u32, value: u8) -> RollbackCommand {
    command_from(tick, 0, tick as u64, value)
}

fn command_from(tick: u32, source: u64, sequence: u64, value: u8) -> RollbackCommand {
    RollbackCommand {
        tick,
        source,
        sequence,
        payload: vec![value],
    }
}

#[test]
fn saves_and_prunes_old_subsystem_states() {
    let mut rollback = DeterministicRollbackBuffer::default().with_capacity_ticks(2);

    rollback.save_state(10, [10]);
    rollback.save_state(11, [11]);
    rollback.save_state(12, [12]);
    rollback.save_state(13, [13]);

    assert_eq!(rollback.state(10), None);
    assert_eq!(rollback.state(11), Some([11].as_slice()));
    assert_eq!(rollback.state(13), Some([13].as_slice()));
}

#[test]
fn builds_replay_from_authoritative_tick_to_current_tick() {
    let mut rollback = DeterministicRollbackBuffer::default();
    rollback.save_state(98, [1]);
    let commands = [
        command(98, 1),
        command(99, 2),
        command(100, 3),
        command(101, 4),
    ];

    let replay = rollback.build_replay(98, 100, commands).unwrap();

    assert_eq!(replay.from_tick, 98);
    assert_eq!(replay.to_tick, 100);
    assert_eq!(replay.initial_state, [1]);
    assert_eq!(
        replay
            .commands
            .iter()
            .map(|command| command.tick)
            .collect::<Vec<_>>(),
        [99, 100]
    );
}

#[test]
fn replay_commands_are_sorted_and_deduplicated_by_stable_identity() {
    let mut rollback = DeterministicRollbackBuffer::default();
    rollback.save_state(98, [1]);
    let commands = [
        command_from(100, 2, 1, 20),
        command_from(99, 1, 2, 10),
        command_from(99, 1, 1, 5),
        command_from(99, 1, 1, 5),
    ];

    let replay = rollback.build_replay(98, 100, commands).unwrap();

    assert_eq!(
        replay
            .commands
            .iter()
            .map(|command| (command.tick, command.source, command.sequence))
            .collect::<Vec<_>>(),
        [(99, 1, 1), (99, 1, 2), (100, 2, 1)]
    );
}

#[test]
fn replay_requires_saved_authoritative_state() {
    let rollback = DeterministicRollbackBuffer::default();

    assert_eq!(
        rollback.build_replay(10, 12, [command(11, 1)]),
        Err(RollbackReplayError::MissingState)
    );
}

#[test]
fn replay_rejects_conflicting_duplicate_command_identity() {
    let mut rollback = DeterministicRollbackBuffer::default();
    rollback.save_state(98, [1]);

    assert_eq!(
        rollback.build_replay(
            98,
            100,
            [command_from(99, 7, 1, 10), command_from(99, 7, 1, 11),],
        ),
        Err(RollbackReplayError::DuplicateCommand)
    );
}

#[test]
fn replay_bytes_applies_commands_in_order() {
    let replay = RollbackReplay {
        from_tick: 1,
        to_tick: 3,
        initial_state: vec![10],
        commands: vec![command(2, 3), command(3, 7)],
    };

    let state = replay_bytes(&replay, |state, command| {
        state[0] = state[0].saturating_add(command.payload[0]);
    });

    assert_eq!(state, [20]);
}

#[test]
fn clear_removes_all_subsystem_history() {
    let mut rollback = DeterministicRollbackBuffer::default();
    rollback.save_state(1, [1]);

    rollback.clear();

    assert!(rollback.is_empty());
}

#[test]
fn policy_accepts_only_commands_inside_rollback_window() {
    let policy = RollbackPolicy {
        max_rollback_ticks: 12,
        commit_delay_ticks: 4,
    };

    assert_eq!(
        policy.classify_command(160, 148),
        RollbackCommandDecision::Replay
    );
    assert_eq!(
        policy.classify_command(160, 147),
        RollbackCommandDecision::TooOld
    );
    assert_eq!(
        policy.classify_command(160, 161),
        RollbackCommandDecision::FromFuture
    );
}

#[test]
fn late_command_replay_restores_tick_before_command() {
    let mut rollback = DeterministicRollbackBuffer::default();
    rollback.save_state(119, [9]);
    let commands = [command(119, 1), command(120, 2), command(121, 3)];

    let replay = rollback
        .build_late_command_replay(RollbackPolicy::default(), 121, 120, commands)
        .unwrap();

    assert_eq!(replay.from_tick, 119);
    assert_eq!(replay.to_tick, 121);
    assert_eq!(
        replay
            .commands
            .iter()
            .map(|command| command.tick)
            .collect::<Vec<_>>(),
        [120, 121]
    );
}

#[test]
fn late_command_replay_rejects_too_old_future_or_missing_state() {
    let rollback = DeterministicRollbackBuffer::default();
    let policy = RollbackPolicy {
        max_rollback_ticks: 2,
        commit_delay_ticks: 1,
    };

    assert_eq!(
        rollback.build_late_command_replay(policy, 10, 7, [command(7, 1)]),
        Err(RollbackReplayError::TooOld)
    );
    assert_eq!(
        rollback.build_late_command_replay(policy, 10, 11, [command(11, 1)]),
        Err(RollbackReplayError::FromFuture)
    );
    assert_eq!(
        rollback.build_late_command_replay(policy, 10, 9, [command(9, 1)]),
        Err(RollbackReplayError::MissingState)
    );
}

#[test]
fn late_command_replay_rejects_tick_zero_without_pretick_state() {
    let mut rollback = DeterministicRollbackBuffer::default();
    rollback.save_state(0, [0]);

    assert_eq!(
        rollback.build_late_command_replay(RollbackPolicy::default(), 0, 0, [command(0, 1)]),
        Err(RollbackReplayError::MissingState)
    );
}

#[test]
fn policy_keeps_combat_events_pending_until_commit_delay_passes() {
    let policy = RollbackPolicy {
        max_rollback_ticks: 12,
        commit_delay_ticks: 4,
    };

    assert_eq!(policy.pending_until(120), 124);
    assert!(!policy.event_is_final(123, 120));
    assert!(policy.event_is_final(124, 120));
}

#[test]
fn committed_tick_marks_the_rewrite_boundary() {
    let policy = RollbackPolicy {
        max_rollback_ticks: 8,
        commit_delay_ticks: 3,
    };

    assert_eq!(policy.committed_tick(100), 97);
    assert!(!policy.tick_is_provisional(100, 97));
    assert!(policy.tick_is_provisional(100, 98));
    assert!(policy.tick_is_provisional(100, 100));
    assert!(!policy.tick_is_provisional(100, 101));
}

#[test]
fn provisional_state_is_used_for_live_hit_detection() {
    let mut domain = CommittedRollbackDomain::new(
        RollbackDomainId(1),
        100,
        // [paladin_x, projectile_x, mage_was_hit, paladin_was_hit]
        [0, 0, 0, 0],
        RollbackPolicy {
            max_rollback_ticks: 8,
            commit_delay_ticks: 4,
        },
    );
    domain
        .insert_command(
            103,
            RollbackCommand {
                tick: 101,
                source: 1,
                sequence: 1,
                payload: vec![1, 5],
            },
        )
        .unwrap();
    domain
        .insert_command(
            103,
            RollbackCommand {
                tick: 102,
                source: 1,
                sequence: 2,
                payload: vec![2, 5],
            },
        )
        .unwrap();

    let replay = domain.rebuild_provisional(103, apply_projectile_example);

    assert_eq!(domain.committed_state(), [0, 0, 0, 0]);
    assert_eq!(replay.provisional_state, [5, 5, 0, 1]);
}

#[test]
fn corrected_death_removes_later_projectile_spawn_without_manual_undo() {
    let projectile = StableEntityId::from_raw(9001);
    let mut domain = CommittedRollbackDomain::new(
        RollbackDomainId(7),
        100,
        // [alive, projectile_count]
        [1, 0],
        RollbackPolicy {
            max_rollback_ticks: 8,
            commit_delay_ticks: 4,
        },
    );
    domain
        .insert_command(
            103,
            RollbackCommand {
                tick: 103,
                source: 1,
                sequence: 1,
                payload: vec![11],
            },
        )
        .unwrap();
    let first = domain.rebuild_provisional(103, apply_lifecycle_example);

    assert_eq!(first.provisional_state, [1, 1]);
    assert!(
        first
            .outputs
            .lifecycles
            .get(&projectile)
            .is_some_and(|lifecycle| lifecycle.is_alive_at(103))
    );

    domain
        .insert_command(
            103,
            RollbackCommand {
                tick: 102,
                source: 2,
                sequence: 1,
                payload: vec![10],
            },
        )
        .unwrap();
    let corrected = domain.rebuild_provisional(103, apply_lifecycle_example);

    assert_eq!(corrected.provisional_state, [0, 0]);
    assert!(!corrected.outputs.lifecycles.contains_key(&projectile));
    assert_eq!(corrected.cue_diff.removed.len(), 1);
    assert_eq!(corrected.cue_diff.removed[0].kind, "projectile_spawned");
}

#[test]
fn projectile_spawn_before_corrected_death_remains_valid() {
    let projectile = StableEntityId::from_raw(9001);
    let mut domain = CommittedRollbackDomain::new(
        RollbackDomainId(8),
        100,
        [1, 0],
        RollbackPolicy {
            max_rollback_ticks: 8,
            commit_delay_ticks: 4,
        },
    );
    domain
        .insert_command(
            103,
            RollbackCommand {
                tick: 101,
                source: 1,
                sequence: 1,
                payload: vec![11],
            },
        )
        .unwrap();
    domain
        .insert_command(
            103,
            RollbackCommand {
                tick: 102,
                source: 2,
                sequence: 1,
                payload: vec![10],
            },
        )
        .unwrap();

    let replay = domain.rebuild_provisional(103, apply_lifecycle_example);

    assert_eq!(replay.provisional_state, [0, 1]);
    assert!(
        replay
            .outputs
            .lifecycles
            .get(&projectile)
            .is_some_and(|lifecycle| lifecycle.is_alive_at(101))
    );
}

#[test]
fn promote_committed_prunes_commands_and_rejects_old_arguments() {
    let mut domain = CommittedRollbackDomain::new(
        RollbackDomainId(9),
        100,
        [1, 0],
        RollbackPolicy {
            max_rollback_ticks: 8,
            commit_delay_ticks: 2,
        },
    );
    domain
        .insert_command(
            104,
            RollbackCommand {
                tick: 101,
                source: 1,
                sequence: 1,
                payload: vec![11],
            },
        )
        .unwrap();
    domain
        .insert_command(
            104,
            RollbackCommand {
                tick: 103,
                source: 2,
                sequence: 1,
                payload: vec![10],
            },
        )
        .unwrap();

    domain.promote_committed(104, apply_lifecycle_example);

    assert_eq!(domain.committed_tick(), 102);
    assert_eq!(domain.committed_state(), [1, 1]);
    assert_eq!(
        domain.insert_command(
            104,
            RollbackCommand {
                tick: 101,
                source: 1,
                sequence: 2,
                payload: vec![10],
            },
        ),
        Err(RollbackReplayError::AlreadyCommitted)
    );
    assert_eq!(domain.commands().len(), 1);
    assert_eq!(domain.commands()[0].tick, 103);
}

#[test]
fn domain_rejects_duplicate_command_identity() {
    let mut domain = CommittedRollbackDomain::new(
        RollbackDomainId(10),
        100,
        [0],
        RollbackPolicy {
            max_rollback_ticks: 8,
            commit_delay_ticks: 2,
        },
    );
    let command = RollbackCommand {
        tick: 101,
        source: 7,
        sequence: 42,
        payload: vec![1],
    };

    assert_eq!(domain.insert_command(101, command.clone()), Ok(()));
    assert_eq!(
        domain.insert_command(101, command),
        Err(RollbackReplayError::DuplicateCommand)
    );
    assert_eq!(domain.commands().len(), 1);
}

#[allow(clippy::ptr_arg)]
fn apply_projectile_example(
    state: &mut Vec<u8>,
    command: &RollbackCommand,
    outputs: &mut RollbackDomainOutputs,
) {
    match command.payload.as_slice() {
        [1, x] => state[0] = *x,
        [2, x] => {
            state[1] = *x;
            if state[0] == state[1] {
                state[3] = 1;
                outputs.cue(command.tick, 1, "paladin_blocked_projectile", []);
            } else {
                state[2] = 1;
                outputs.cue(command.tick, 1, "mage_hit", []);
            }
        }
        _ => {}
    }
}

#[allow(clippy::ptr_arg)]
fn apply_lifecycle_example(
    state: &mut Vec<u8>,
    command: &RollbackCommand,
    outputs: &mut RollbackDomainOutputs,
) {
    let projectile = StableEntityId::from_raw(9001);
    match command.payload.as_slice() {
        [10] => {
            state[0] = 0;
            outputs.cue(command.tick, 1, "character_died", []);
        }
        [11] if state[0] == 1 => {
            state[1] = state[1].saturating_add(1);
            outputs.spawn_entity(projectile, command.tick);
            outputs.cue(command.tick, 2, "projectile_spawned", [1]);
        }
        [11] => {
            outputs.cue(command.tick, 3, "shoot_rejected_dead", []);
        }
        _ => {}
    }
}
