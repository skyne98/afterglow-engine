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
fn interest_map_batch_filters_snapshots_for_many_players() {
    let mut world = ReplicationWorld::default();
    let first = StableEntityId::from_raw(1);
    let second = StableEntityId::from_raw(2);
    let hidden = StableEntityId::from_raw(3);
    world.set_field(first, "name", b"first".to_vec());
    world.set_field(second, "name", b"second".to_vec());
    world.set_field(hidden, "name", b"hidden".to_vec());
    let snapshot = world.snapshot(7);

    let mut interest = InterestMap::default();
    interest.set_entity_chunk(first, ChunkId::from_raw(10));
    interest.set_entity_chunk(second, ChunkId::from_raw(20));
    interest.set_entity_chunk(hidden, ChunkId::from_raw(30));
    interest.set_player_chunks(NetworkPlayerId(1), [ChunkId::from_raw(10)]);
    interest.set_player_chunks(NetworkPlayerId(2), [ChunkId::from_raw(20)]);

    let filtered = interest.filter_snapshots([NetworkPlayerId(1), NetworkPlayerId(2)], &snapshot);

    assert_eq!(filtered[&NetworkPlayerId(1)].tick, 7);
    assert_eq!(filtered[&NetworkPlayerId(1)].entities[0].entity, first);
    assert_eq!(filtered[&NetworkPlayerId(2)].entities[0].entity, second);
}

#[test]
fn interest_map_builds_chunk_snapshot_fanout_without_per_player_entity_copies() {
    let mut world = ReplicationWorld::default();
    let first = StableEntityId::from_raw(1);
    let second = StableEntityId::from_raw(2);
    world.set_field(first, "name", b"first".to_vec());
    world.set_field(second, "name", b"second".to_vec());
    let snapshot = world.snapshot(8);

    let mut interest = InterestMap::default();
    interest.set_entity_chunk(first, ChunkId::from_raw(10));
    interest.set_entity_chunk(second, ChunkId::from_raw(20));
    interest.set_player_chunks(NetworkPlayerId(1), [ChunkId::from_raw(10)]);
    interest.set_player_chunks(
        NetworkPlayerId(2),
        [ChunkId::from_raw(10), ChunkId::from_raw(20)],
    );

    let fanout =
        interest.snapshot_chunk_fanout([NetworkPlayerId(1), NetworkPlayerId(2)], &snapshot);

    assert_eq!(fanout.tick, 8);
    assert_eq!(
        fanout.chunk_players[&ChunkId::from_raw(10)],
        [NetworkPlayerId(1), NetworkPlayerId(2)]
    );
    assert_eq!(fanout.chunks[&ChunkId::from_raw(10)][0].entity, first);
    assert_eq!(fanout.chunks[&ChunkId::from_raw(20)][0].entity, second);
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
fn interest_map_routes_removals_after_entity_chunk_is_removed() {
    let removed = StableEntityId::from_raw(1);
    let delta = WorldDelta {
        from_tick: 1,
        to_tick: 2,
        changes: Vec::new(),
        removed: vec![removed],
    };
    let mut interest = InterestMap::default();
    interest.set_entity_chunk(removed, ChunkId::from_raw(1));
    interest.set_player_chunks(NetworkPlayerId(1), [ChunkId::from_raw(1)]);

    interest.remove_entity(removed);

    let filtered = interest.filter_delta(NetworkPlayerId(1), &delta);
    let fanout = interest.delta_chunk_ref_fanout([NetworkPlayerId(1)], &delta);

    assert_eq!(filtered.removed, [removed]);
    assert_eq!(fanout.chunks[&ChunkId::from_raw(1)].removed, [removed]);
}

#[test]
fn interest_map_clears_routed_removed_entity_chunks() {
    let removed = StableEntityId::from_raw(1);
    let unknown = StableEntityId::from_raw(99);
    let delta = WorldDelta {
        from_tick: 1,
        to_tick: 2,
        changes: Vec::new(),
        removed: vec![removed],
    };
    let mut interest = InterestMap::default();
    interest.set_entity_chunk(removed, ChunkId::from_raw(1));
    interest.set_player_chunks(NetworkPlayerId(1), [ChunkId::from_raw(1)]);
    interest.remove_entity(removed);

    interest.clear_removed_entities([unknown, removed]);

    let filtered = interest.filter_delta(NetworkPlayerId(1), &delta);
    let fanout = interest.delta_chunk_ref_fanout([NetworkPlayerId(1)], &delta);

    assert!(filtered.removed.is_empty());
    assert!(fanout.chunks[&ChunkId::from_raw(1)].removed.is_empty());
}

#[test]
fn interest_map_automatically_clears_removals_routed_by_batch_fanout() {
    let removed = StableEntityId::from_raw(1);
    let delta = WorldDelta {
        from_tick: 1,
        to_tick: 2,
        changes: Vec::new(),
        removed: vec![removed],
    };
    let mut interest = InterestMap::default();
    interest.set_entity_chunk(removed, ChunkId::from_raw(1));
    interest.set_player_chunks(NetworkPlayerId(1), [ChunkId::from_raw(1)]);
    interest.remove_entity(removed);

    let fanout = interest.delta_chunk_ref_fanout([NetworkPlayerId(1)], &delta);
    let after_cleanup = interest.filter_delta(NetworkPlayerId(1), &delta);

    assert_eq!(fanout.chunks[&ChunkId::from_raw(1)].removed, [removed]);
    assert!(after_cleanup.removed.is_empty());
}

#[test]
fn interest_map_automatically_clears_removals_routed_by_batch_filter() {
    let removed = StableEntityId::from_raw(1);
    let delta = WorldDelta {
        from_tick: 1,
        to_tick: 2,
        changes: Vec::new(),
        removed: vec![removed],
    };
    let mut interest = InterestMap::default();
    interest.set_entity_chunk(removed, ChunkId::from_raw(1));
    interest.set_player_chunks(NetworkPlayerId(1), [ChunkId::from_raw(1)]);
    interest.remove_entity(removed);

    let filtered = interest.filter_deltas([NetworkPlayerId(1)], &delta);
    let after_cleanup = interest.filter_delta(NetworkPlayerId(1), &delta);

    assert_eq!(filtered[&NetworkPlayerId(1)].removed, [removed]);
    assert!(after_cleanup.removed.is_empty());
}

#[test]
fn interest_map_keeps_unrouted_removals_after_partial_batch_cleanup() {
    let visible_removed = StableEntityId::from_raw(1);
    let hidden_removed = StableEntityId::from_raw(2);
    let delta = WorldDelta {
        from_tick: 1,
        to_tick: 2,
        changes: Vec::new(),
        removed: vec![visible_removed, hidden_removed],
    };
    let mut interest = InterestMap::default();
    interest.set_entity_chunk(visible_removed, ChunkId::from_raw(1));
    interest.set_entity_chunk(hidden_removed, ChunkId::from_raw(2));
    interest.set_player_chunks(NetworkPlayerId(1), [ChunkId::from_raw(1)]);
    interest.remove_entity(visible_removed);
    interest.remove_entity(hidden_removed);

    let fanout = interest.delta_chunk_ref_fanout([NetworkPlayerId(1)], &delta);
    interest.set_player_chunks(NetworkPlayerId(2), [ChunkId::from_raw(2)]);
    let later_visible = interest.filter_delta(NetworkPlayerId(2), &delta);

    assert_eq!(
        fanout.chunks[&ChunkId::from_raw(1)].removed,
        [visible_removed]
    );
    assert_eq!(later_visible.removed, [hidden_removed]);
}

#[test]
fn interest_map_does_not_clear_removals_when_no_players_receive_them() {
    let removed = StableEntityId::from_raw(1);
    let delta = WorldDelta {
        from_tick: 1,
        to_tick: 2,
        changes: Vec::new(),
        removed: vec![removed],
    };
    let mut interest = InterestMap::default();
    interest.set_entity_chunk(removed, ChunkId::from_raw(1));
    interest.remove_entity(removed);

    let fanout = interest.delta_chunk_ref_fanout([NetworkPlayerId(1)], &delta);
    interest.set_player_chunks(NetworkPlayerId(1), [ChunkId::from_raw(1)]);
    let later_visible = interest.filter_delta(NetworkPlayerId(1), &delta);

    assert!(fanout.chunks.is_empty());
    assert_eq!(later_visible.removed, [removed]);
}

#[test]
fn interest_map_routes_old_removal_to_old_chunk_after_respawn() {
    let entity = StableEntityId::from_raw(1);
    let delta = WorldDelta {
        from_tick: 1,
        to_tick: 2,
        changes: Vec::new(),
        removed: vec![entity],
    };
    let mut interest = InterestMap::default();
    interest.set_entity_chunk(entity, ChunkId::from_raw(1));
    interest.set_player_chunks(NetworkPlayerId(1), [ChunkId::from_raw(1)]);
    interest.set_player_chunks(NetworkPlayerId(2), [ChunkId::from_raw(2)]);
    interest.remove_entity(entity);

    interest.set_entity_chunk(entity, ChunkId::from_raw(2));

    assert_eq!(
        interest.filter_delta(NetworkPlayerId(1), &delta).removed,
        [entity]
    );
    assert_eq!(
        interest.filter_delta(NetworkPlayerId(2), &delta).removed,
        Vec::<StableEntityId>::new()
    );
    assert!(interest.can_see_entity(NetworkPlayerId(2), entity));
}

#[test]
fn interest_map_can_keep_routed_removals_until_explicit_ack_cleanup() {
    let removed = StableEntityId::from_raw(1);
    let delta = WorldDelta {
        from_tick: 1,
        to_tick: 2,
        changes: Vec::new(),
        removed: vec![removed],
    };
    let mut interest = InterestMap::default();
    interest.set_cleanup_routed_removals(false);
    interest.set_entity_chunk(removed, ChunkId::from_raw(1));
    interest.set_player_chunks(NetworkPlayerId(1), [ChunkId::from_raw(1)]);
    interest.remove_entity(removed);

    let fanout = interest.delta_chunk_ref_fanout([NetworkPlayerId(1)], &delta);
    let retained = interest.filter_delta(NetworkPlayerId(1), &delta);
    interest.clear_removed_entities([removed]);
    let cleared = interest.filter_delta(NetworkPlayerId(1), &delta);

    assert_eq!(fanout.chunks[&ChunkId::from_raw(1)].removed, [removed]);
    assert_eq!(retained.removed, [removed]);
    assert!(cleared.removed.is_empty());
}

#[test]
fn interest_map_batch_filters_deltas_for_many_players() {
    let first = StableEntityId::from_raw(1);
    let second = StableEntityId::from_raw(2);
    let delta = WorldDelta {
        from_tick: 4,
        to_tick: 5,
        changes: vec![
            EntityDelta {
                entity: first,
                changed: vec![FieldValue {
                    name: "hp".into(),
                    value: vec![10],
                }],
                removed: Vec::new(),
            },
            EntityDelta {
                entity: second,
                changed: vec![FieldValue {
                    name: "hp".into(),
                    value: vec![20],
                }],
                removed: Vec::new(),
            },
        ],
        removed: vec![first, second],
    };
    let mut interest = InterestMap::default();
    interest.set_entity_chunk(first, ChunkId::from_raw(1));
    interest.set_entity_chunk(second, ChunkId::from_raw(2));
    interest.set_player_chunks(NetworkPlayerId(1), [ChunkId::from_raw(1)]);
    interest.set_player_chunks(NetworkPlayerId(2), [ChunkId::from_raw(2)]);

    let filtered = interest.filter_deltas([NetworkPlayerId(1), NetworkPlayerId(2)], &delta);

    assert_eq!(filtered[&NetworkPlayerId(1)].changes[0].entity, first);
    assert_eq!(filtered[&NetworkPlayerId(1)].removed, [first]);
    assert_eq!(filtered[&NetworkPlayerId(2)].changes[0].entity, second);
    assert_eq!(filtered[&NetworkPlayerId(2)].removed, [second]);
}

#[test]
fn interest_map_builds_chunk_delta_fanout_without_per_player_entity_copies() {
    let first = StableEntityId::from_raw(1);
    let second = StableEntityId::from_raw(2);
    let delta = WorldDelta {
        from_tick: 8,
        to_tick: 9,
        changes: vec![EntityDelta {
            entity: first,
            changed: vec![FieldValue {
                name: "hp".into(),
                value: vec![10],
            }],
            removed: Vec::new(),
        }],
        removed: vec![second],
    };
    let mut interest = InterestMap::default();
    interest.set_entity_chunk(first, ChunkId::from_raw(1));
    interest.set_entity_chunk(second, ChunkId::from_raw(2));
    interest.set_player_chunks(NetworkPlayerId(1), [ChunkId::from_raw(1)]);
    interest.set_player_chunks(
        NetworkPlayerId(2),
        [ChunkId::from_raw(1), ChunkId::from_raw(2)],
    );

    let fanout = interest.delta_chunk_fanout([NetworkPlayerId(1), NetworkPlayerId(2)], &delta);

    assert_eq!(fanout.from_tick, 8);
    assert_eq!(fanout.to_tick, 9);
    assert_eq!(
        fanout.chunks[&ChunkId::from_raw(1)].changes[0].entity,
        first
    );
    assert_eq!(fanout.chunks[&ChunkId::from_raw(2)].removed, [second]);
    assert_eq!(
        fanout.chunk_players[&ChunkId::from_raw(1)],
        [NetworkPlayerId(1), NetworkPlayerId(2)]
    );
}

#[test]
fn interest_map_deduplicates_duplicate_players_in_batch_outputs() {
    let mut world = ReplicationWorld::default();
    let entity = StableEntityId::from_raw(1);
    world.set_field(entity, "name", b"visible".to_vec());
    let snapshot = world.snapshot(1);
    let mut interest = InterestMap::default();
    interest.set_entity_chunk(entity, ChunkId::from_raw(1));
    interest.set_player_chunks(NetworkPlayerId(1), [ChunkId::from_raw(1)]);

    let filtered = interest.filter_snapshots([NetworkPlayerId(1), NetworkPlayerId(1)], &snapshot);
    let fanout =
        interest.snapshot_chunk_ref_fanout([NetworkPlayerId(1), NetworkPlayerId(1)], &snapshot);

    assert_eq!(filtered[&NetworkPlayerId(1)].entities.len(), 1);
    assert_eq!(
        fanout.chunk_players[&ChunkId::from_raw(1)],
        [NetworkPlayerId(1)]
    );
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
