use super::*;
use crate::network::replication::{FieldValue, ReplicationWorld};

#[test]
fn interest_map_filters_snapshot_by_player_chunks() {
    let mut world = ReplicationWorld::default();
    let visible = StableEntityId::from_raw(1);
    let hidden = StableEntityId::from_raw(2);
    world.set_field(visible, "name", b"visible".to_vec());
    world.set_field(hidden, "name", b"hidden".to_vec());
    let snapshot = world.snapshot(3);

    let mut interest = InterestMap::default();
    interest.set_entity_chunk(visible, ChunkId::from_raw(10));
    interest.set_entity_chunk(hidden, ChunkId::from_raw(20));
    interest.set_player_chunks(NetworkPlayerId(1), [ChunkId::from_raw(10)]);

    let filtered = interest.filter_snapshot(NetworkPlayerId(1), &snapshot);

    assert_eq!(filtered.tick, 3);
    assert_eq!(filtered.entities.len(), 1);
    assert_eq!(filtered.entities[0].entity, visible);
}

#[test]
fn interest_map_filters_delta_changes_and_removals() {
    let visible = StableEntityId::from_raw(1);
    let hidden = StableEntityId::from_raw(2);
    let delta = WorldDelta {
        from_tick: 1,
        to_tick: 2,
        changes: vec![
            EntityDelta {
                entity: visible,
                changed: vec![FieldValue {
                    name: "hp".into(),
                    value: vec![10],
                }],
                removed: Vec::new(),
            },
            EntityDelta {
                entity: hidden,
                changed: vec![FieldValue {
                    name: "hp".into(),
                    value: vec![10],
                }],
                removed: Vec::new(),
            },
        ],
        removed: vec![visible, hidden],
    };
    let mut interest = InterestMap::default();
    interest.set_entity_chunk(visible, ChunkId::from_raw(1));
    interest.set_entity_chunk(hidden, ChunkId::from_raw(2));
    interest.set_player_chunks(NetworkPlayerId(1), [ChunkId::from_raw(1)]);

    let filtered = interest.filter_delta(NetworkPlayerId(1), &delta);

    assert_eq!(filtered.from_tick, 1);
    assert_eq!(filtered.to_tick, 2);
    assert_eq!(filtered.changes.len(), 1);
    assert_eq!(filtered.changes[0].entity, visible);
    assert_eq!(filtered.removed, [visible]);
}

#[test]
fn unknown_entity_or_player_is_not_visible() {
    let mut interest = InterestMap::default();
    let entity = StableEntityId::from_raw(1);

    interest.set_entity_chunk(entity, ChunkId::from_raw(1));

    assert!(!interest.can_see_entity(NetworkPlayerId(99), entity));
    assert!(!interest.can_see_entity(NetworkPlayerId(1), StableEntityId::from_raw(99)));
}

#[test]
fn moving_entity_between_chunks_updates_visibility() {
    let mut interest = InterestMap::default();
    let entity = StableEntityId::from_raw(1);
    let player = NetworkPlayerId(1);
    interest.set_player_chunks(player, [ChunkId::from_raw(1)]);

    interest.set_entity_chunk(entity, ChunkId::from_raw(1));
    assert!(interest.can_see_entity(player, entity));

    interest.set_entity_chunk(entity, ChunkId::from_raw(2));
    assert!(!interest.can_see_entity(player, entity));
}
