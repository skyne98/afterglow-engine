use super::*;

#[test]
fn snapshot_captures_entities_in_stable_order() {
    let mut world = ReplicationWorld::default();
    world.set_field(StableEntityId::from_raw(2), "hp", [10]);
    world.set_field(StableEntityId::from_raw(1), "pos", [1, 2, 3]);

    let snapshot = world.snapshot(7);

    assert_eq!(snapshot.tick, 7);
    assert_eq!(
        snapshot
            .entities
            .iter()
            .map(|entity| entity.entity)
            .collect::<Vec<_>>(),
        [StableEntityId::from_raw(1), StableEntityId::from_raw(2)]
    );
}

#[test]
fn delta_tracks_changed_removed_and_deleted_state() {
    let mut world = ReplicationWorld::default();
    let player = StableEntityId::from_raw(1);
    let item = StableEntityId::from_raw(2);
    world.set_field(player, "pos", [0]);
    world.set_field(player, "hp", [10]);
    world.set_field(item, "kind", b"potion".to_vec());
    let baseline = world.snapshot(1);

    world.set_field(player, "pos", [4]);
    world.remove_field(player, "hp");
    world.remove_entity(item);
    let delta = world.delta_since(&baseline, 2);

    assert_eq!(delta.from_tick, 1);
    assert_eq!(delta.to_tick, 2);
    assert_eq!(delta.removed, [item]);
    assert_eq!(
        delta.changes,
        [EntityDelta {
            entity: player,
            changed: vec![FieldValue {
                name: "pos".into(),
                value: vec![4],
            }],
            removed: vec!["hp".into()],
        }]
    );
}

#[test]
fn applying_snapshot_and_delta_recreates_authoritative_world() {
    let mut server = ReplicationWorld::default();
    let player = StableEntityId::from_raw(1);
    server.set_field(player, "pos", [0, 0, 0]);
    let baseline = server.snapshot(10);

    server.set_field(player, "pos", [1, 2, 3]);
    server.set_field(player, "animation", b"run".to_vec());
    let delta = server.delta_since(&baseline, 11);

    let mut client = ReplicationWorld::default();
    client.apply_snapshot(&baseline);
    client.apply_delta(&delta);

    assert_eq!(client.snapshot(11), server.snapshot(11));
    assert_eq!(
        client.entity(player).unwrap().field("pos"),
        Some([1, 2, 3].as_slice())
    );
    assert_eq!(
        client.entity(player).unwrap().field("animation"),
        Some(b"run".as_slice())
    );
}

#[test]
fn dirty_delta_only_reports_touched_entities() {
    let mut world = ReplicationWorld::default();
    let stable = StableEntityId::from_raw(1);
    let changed = StableEntityId::from_raw(2);
    world.set_field(stable, "pos", [0]);
    world.set_field(changed, "pos", [0]);
    world.clear_changes();
    let baseline = world.snapshot(1);

    world.set_field(changed, "pos", [1]);
    let delta = world.dirty_delta_since(&baseline, 2);

    assert_eq!(delta.changes.len(), 1);
    assert_eq!(delta.changes[0].entity, changed);
}

#[test]
fn dirty_delta_tracks_removed_entities() {
    let mut world = ReplicationWorld::default();
    let removed = StableEntityId::from_raw(1);
    world.set_field(removed, "state", [1]);
    world.clear_changes();
    let baseline = world.snapshot(1);

    world.remove_entity(removed);
    let delta = world.dirty_delta_since(&baseline, 2);

    assert_eq!(delta.removed, [removed]);
    assert!(delta.changes.is_empty());
}

#[test]
fn empty_delta_has_no_changes() {
    let mut world = ReplicationWorld::default();
    world.set_field(StableEntityId::from_raw(1), "state", [1]);
    let baseline = world.snapshot(1);

    let delta = world.delta_since(&baseline, 2);

    assert!(delta.changes.is_empty());
    assert!(delta.removed.is_empty());
}
