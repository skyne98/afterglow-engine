use super::*;
use crate::core::identity::{Replicated, StableEntityId, StableEntityRegistry};
use bevy::prelude::*;

#[derive(Component, Clone, Debug, Eq, PartialEq)]
struct EdgeProjectile {
    owner: StableEntityId,
}

#[derive(Component, Clone, Debug, Eq, PartialEq)]
struct EdgeShield {
    strength: i32,
}

impl Replicate for EdgeProjectile {
    const REPLICATION_NAME: &'static str = "tests::rollback_ecs_edge::EdgeProjectile";
}

impl Replicate for EdgeShield {
    const REPLICATION_NAME: &'static str = "tests::rollback_ecs_edge::EdgeShield";
}

#[derive(Message, Clone, Debug, Eq, PartialEq)]
struct EdgePulse {
    tick: u32,
    id: u8,
}

impl ReplicatedMessage for EdgePulse {
    fn tick(&self) -> u32 {
        self.tick
    }
}

fn network_only_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, crate::network::AfterglowNetworkPlugin));
    app.replicate(component::<EdgeProjectile>())
        .replicate(component::<EdgeShield>())
        .replicate(message::<EdgePulse>());
    app
}

#[test]
fn network_only_save_assigns_stable_ids_before_capturing_replicated_state() {
    let mut app = network_only_app();
    app.world_mut().spawn((
        Replicated,
        EdgeProjectile {
            owner: StableEntityId::from_raw(91),
        },
    ));

    app.world_mut().save_replicated_state(1);

    let snapshot = app
        .world()
        .resource::<ReplicatedComponentHistory<EdgeProjectile>>()
        .snapshot(1)
        .unwrap();
    assert_eq!(snapshot.values().len(), 1);
    assert!(snapshot.values().keys().next().unwrap().is_valid());
}

#[test]
fn restore_removes_replicated_components_without_stable_ids() {
    let mut app = network_only_app();
    app.world_mut().save_replicated_state(1);
    let stale_entity = app
        .world_mut()
        .spawn((
            Replicated,
            EdgeProjectile {
                owner: StableEntityId::from_raw(94),
            },
        ))
        .id();

    app.world_mut().restore_replicated_state(1).unwrap();

    assert!(app.world().get_entity(stale_entity).is_err());
}

#[test]
fn save_replaces_invalid_stable_ids_before_capturing_replicated_state() {
    let mut app = network_only_app();
    app.world_mut().spawn((
        StableEntityId::INVALID,
        Replicated,
        EdgeProjectile {
            owner: StableEntityId::from_raw(95),
        },
    ));

    app.world_mut().save_replicated_state(1);

    let snapshot = app
        .world()
        .resource::<ReplicatedComponentHistory<EdgeProjectile>>()
        .snapshot(1)
        .unwrap();
    assert_eq!(snapshot.values().len(), 1);
    assert!(snapshot.values().keys().next().unwrap().is_valid());
}

#[test]
fn save_uses_canonical_component_when_duplicate_stable_ids_exist() {
    let mut app = network_only_app();
    let duplicate = StableEntityId::from_raw(96);
    app.world_mut().spawn((
        duplicate,
        Replicated,
        EdgeProjectile {
            owner: StableEntityId::from_raw(1),
        },
    ));
    app.world_mut().spawn((
        duplicate,
        Replicated,
        EdgeProjectile {
            owner: StableEntityId::from_raw(2),
        },
    ));

    app.world_mut().save_replicated_state(1);

    let canonical = app
        .world()
        .resource::<StableEntityRegistry>()
        .entity(duplicate)
        .unwrap();
    let expected = app.world().get::<EdgeProjectile>(canonical).unwrap();
    let snapshot = app
        .world()
        .resource::<ReplicatedComponentHistory<EdgeProjectile>>()
        .snapshot(1)
        .unwrap();
    assert_eq!(snapshot.values().get(&duplicate), Some(expected));
}

#[test]
fn replicated_tick_replay_does_not_duplicate_replace_for_replay_pending_messages() {
    let mut app = network_only_app();
    let pulse = EdgePulse { tick: 2, id: 7 };
    app.world_mut()
        .resource_mut::<ReplicatedTimeline<EdgePulse>>()
        .replace_for_replay([(2, pulse.clone())]);

    app.world_mut().run_replicated_tick(2);

    assert_eq!(
        app.world()
            .resource::<ReplicatedTimeline<EdgePulse>>()
            .reissued_pending_collection
            .get(&2)
            .unwrap(),
        &[pulse]
    );
}

#[test]
fn restore_updates_rollback_clock_to_restored_tick() {
    let mut app = network_only_app();
    app.world_mut().save_replicated_state(7);
    app.world_mut()
        .resource_mut::<RollbackReplicationClock>()
        .current_tick = 99;

    app.world_mut().restore_replicated_state(7).unwrap();

    assert_eq!(
        app.world()
            .resource::<RollbackReplicationClock>()
            .current_tick,
        7
    );
}

#[test]
fn restore_collapses_duplicate_stable_replicated_components() {
    let mut app = network_only_app();
    let duplicate = StableEntityId::from_raw(92);
    app.world_mut().spawn((
        duplicate,
        Replicated,
        EdgeProjectile {
            owner: StableEntityId::from_raw(1),
        },
    ));
    app.world_mut().spawn((
        duplicate,
        Replicated,
        EdgeProjectile {
            owner: StableEntityId::from_raw(2),
        },
    ));
    app.world_mut().save_replicated_state(1);

    app.world_mut().restore_replicated_state(1).unwrap();

    assert_eq!(replicated_entities_with(&mut app, duplicate), 1);
    assert_eq!(projectiles_with(&mut app, duplicate), 1);
}

#[test]
fn restore_collapses_multi_component_duplicate_stable_entities() {
    let mut app = network_only_app();
    let duplicate = StableEntityId::from_raw(93);
    app.world_mut().spawn((
        duplicate,
        Replicated,
        EdgeProjectile {
            owner: StableEntityId::from_raw(1),
        },
    ));
    app.world_mut()
        .spawn((duplicate, Replicated, EdgeShield { strength: 5 }));
    app.world_mut().save_replicated_state(1);

    app.world_mut().restore_replicated_state(1).unwrap();

    assert_eq!(replicated_entities_with(&mut app, duplicate), 1);
    assert_eq!(projectiles_with(&mut app, duplicate), 1);
    assert_eq!(shields_with(&mut app, duplicate), 1);
}

fn replicated_entities_with(app: &mut App, stable: StableEntityId) -> usize {
    let world = app.world_mut();
    let mut query = world.query_filtered::<&StableEntityId, With<Replicated>>();
    query
        .iter(world)
        .filter(|entity_stable| **entity_stable == stable)
        .count()
}

fn projectiles_with(app: &mut App, stable: StableEntityId) -> usize {
    let world = app.world_mut();
    let mut query = world.query::<(&StableEntityId, &EdgeProjectile)>();
    query
        .iter(world)
        .filter(|(entity_stable, _)| **entity_stable == stable)
        .count()
}

fn shields_with(app: &mut App, stable: StableEntityId) -> usize {
    let world = app.world_mut();
    let mut query = world.query::<(&StableEntityId, &EdgeShield)>();
    query
        .iter(world)
        .filter(|(entity_stable, _)| **entity_stable == stable)
        .count()
}
