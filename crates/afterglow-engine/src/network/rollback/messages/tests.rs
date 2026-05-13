use super::*;
use crate::core::identity::StableEntityId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Reflect, Serialize, Deserialize)]
enum GameFact {
    Damage { target: StableEntityId, amount: u16 },
    Death { target: StableEntityId },
    Block { target: StableEntityId },
}

fn message(tick: u32, sequence: u64, payload: GameFact) -> RollbackMessage<GameFact> {
    let entities = match &payload {
        GameFact::Damage { target, .. }
        | GameFact::Death { target }
        | GameFact::Block { target } => [*target],
    };
    RollbackMessage::new(
        RollbackMessageId::new(RollbackDomainId(1), tick, sequence),
        payload,
    )
    .with_source_command_tick(tick)
    .with_entities(entities)
}

#[test]
fn provisional_diff_tracks_added_removed_and_changed_facts() {
    let player = StableEntityId::from_raw(10);
    let mut stream = RollbackMessageStream::default();

    let first = stream.replace_provisional([
        message(
            100,
            1,
            GameFact::Damage {
                target: player,
                amount: 25,
            },
        ),
        message(100, 2, GameFact::Death { target: player }),
    ]);

    assert_eq!(first.added.len(), 2);
    assert!(first.removed.is_empty());

    let corrected =
        stream.replace_provisional([message(100, 1, GameFact::Block { target: player })]);

    assert_eq!(corrected.added.len(), 1);
    assert_eq!(corrected.removed.len(), 2);
    assert_eq!(
        corrected.removed,
        [
            RollbackMessageId::new(RollbackDomainId(1), 100, 1),
            RollbackMessageId::new(RollbackDomainId(1), 100, 2),
        ]
    );
    assert!(matches!(corrected.added[0].payload, GameFact::Block { .. }));
}

#[test]
fn committed_stream_promotes_only_final_ticks() {
    let player = StableEntityId::from_raw(20);
    let mut stream = RollbackMessageStream::default();
    stream.replace_provisional([
        message(
            100,
            1,
            GameFact::Damage {
                target: player,
                amount: 10,
            },
        ),
        message(104, 1, GameFact::Death { target: player }),
    ]);

    let commit = stream.commit_through(100);

    assert_eq!(commit.committed_tick, 100);
    assert_eq!(commit.added.len(), 1);
    assert_eq!(stream.committed().count(), 1);
    assert_eq!(stream.provisional().count(), 1);
    assert!(matches!(
        stream.committed().next().unwrap().payload,
        GameFact::Damage { .. }
    ));
}

#[test]
fn corrected_provisional_fact_is_committed_after_the_horizon() {
    let player = StableEntityId::from_raw(30);
    let mut stream = RollbackMessageStream::default();
    stream.replace_provisional([message(100, 1, GameFact::Death { target: player })]);
    stream.replace_provisional([message(100, 1, GameFact::Block { target: player })]);

    let commit = stream.commit_through(100);

    assert_eq!(commit.added.len(), 1);
    assert!(matches!(commit.added[0].payload, GameFact::Block { .. }));
    assert!(
        stream
            .committed_message(RollbackMessageId::new(RollbackDomainId(1), 100, 1))
            .is_some_and(|message| matches!(message.payload, GameFact::Block { .. }))
    );
}

#[test]
fn duplicate_message_ids_keep_last_replay_output() {
    let player = StableEntityId::from_raw(40);
    let mut stream = RollbackMessageStream::default();

    stream.replace_provisional([
        message(
            100,
            1,
            GameFact::Damage {
                target: player,
                amount: 5,
            },
        ),
        message(
            100,
            1,
            GameFact::Damage {
                target: player,
                amount: 9,
            },
        ),
    ]);

    let message = stream
        .provisional_message(RollbackMessageId::new(RollbackDomainId(1), 100, 1))
        .unwrap();
    assert!(matches!(
        message.payload,
        GameFact::Damage { amount: 9, .. }
    ));
    assert_eq!(stream.provisional().count(), 1);
}

#[test]
fn unsorted_duplicate_message_ids_keep_last_input_and_produce_one_added_diff() {
    let player = StableEntityId::from_raw(50);
    let mut stream = RollbackMessageStream::default();

    let diff = stream.replace_provisional([
        message(101, 1, GameFact::Block { target: player }),
        message(
            100,
            1,
            GameFact::Damage {
                target: player,
                amount: 5,
            },
        ),
        message(
            100,
            1,
            GameFact::Damage {
                target: player,
                amount: 9,
            },
        ),
    ]);

    assert_eq!(diff.added.len(), 2);
    assert!(matches!(
        diff.added[0].payload,
        GameFact::Damage { amount: 9, .. }
    ));
    assert!(matches!(diff.added[1].payload, GameFact::Block { .. }));
    assert!(diff.removed.is_empty());
}

#[test]
fn committed_stream_ignores_replayed_messages_at_final_ticks() {
    let player = StableEntityId::from_raw(60);
    let mut stream = RollbackMessageStream::default();
    stream.replace_provisional([message(
        100,
        1,
        GameFact::Damage {
            target: player,
            amount: 5,
        },
    )]);
    stream.commit_through(100);

    let diff = stream.replace_provisional([message(
        100,
        1,
        GameFact::Damage {
            target: player,
            amount: 9,
        },
    )]);
    let commit = stream.commit_through(100);

    assert!(diff.added.is_empty());
    assert!(diff.removed.is_empty());
    assert!(commit.added.is_empty());
    assert_eq!(stream.committed_tick(), Some(100));
    let message = stream
        .committed_message(RollbackMessageId::new(RollbackDomainId(1), 100, 1))
        .unwrap();
    assert!(matches!(
        message.payload,
        GameFact::Damage { amount: 5, .. }
    ));
}

#[test]
fn committed_tick_is_monotonic() {
    let player = StableEntityId::from_raw(70);
    let mut stream = RollbackMessageStream::default();
    stream.replace_provisional([
        message(
            100,
            1,
            GameFact::Damage {
                target: player,
                amount: 5,
            },
        ),
        message(101, 1, GameFact::Block { target: player }),
    ]);

    stream.commit_through(101);
    let commit = stream.commit_through(100);

    assert_eq!(commit.committed_tick, 101);
    assert_eq!(stream.committed_tick(), Some(101));
    assert_eq!(stream.committed().count(), 2);
}
