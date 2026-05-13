use super::*;
use crate::{
    core::identity::{Replicated, StableEntityId},
    network::rollback::RollbackPolicy,
};
use bevy::prelude::*;

#[derive(Component, Clone, Debug, Eq, PartialEq)]
struct RepHealth {
    hp: i32,
}

#[derive(Component, Clone, Debug, Eq, PartialEq)]
struct RepMana {
    mp: i32,
}

#[derive(Component, Clone, Debug, Eq, PartialEq)]
struct RepProjectile {
    owner: StableEntityId,
}

impl Replicate for RepHealth {
    const REPLICATION_NAME: &'static str = "tests::rollback_ecs::RepHealth";
}

impl Replicate for RepMana {
    const REPLICATION_NAME: &'static str = "tests::rollback_ecs::RepMana";
}

impl Replicate for RepProjectile {
    const REPLICATION_NAME: &'static str = "tests::rollback_ecs::RepProjectile";
}

#[derive(Resource, Clone, Debug, Eq, PartialEq)]
struct RepWorldClock {
    hour: u8,
}

impl Replicate for RepWorldClock {
    const REPLICATION_NAME: &'static str = "tests::rollback_ecs::RepWorldClock";
}

#[derive(Message, Clone, Debug, Eq, PartialEq)]
struct Damage {
    tick: u32,
    target: StableEntityId,
    amount: i32,
}

#[derive(Message, Clone, Debug, Eq, PartialEq)]
struct SpawnProjectile {
    tick: u32,
    owner: StableEntityId,
}

impl ReplicatedMessage for Damage {
    fn tick(&self) -> u32 {
        self.tick
    }
}

impl ReplicatedMessage for SpawnProjectile {
    fn tick(&self) -> u32 {
        self.tick
    }
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        crate::core::AfterglowCorePlugin,
        crate::network::AfterglowNetworkPlugin,
    ));
    app.replicate(component::<RepHealth>())
        .replicate(component::<RepMana>())
        .replicate(component::<RepProjectile>())
        .replicate(resource::<RepWorldClock>())
        .replicate(message::<Damage>())
        .replicate(message::<SpawnProjectile>())
        .add_systems(ReplicatedTick, (apply_damage, spawn_projectiles));
    app
}

fn apply_damage(
    mut messages: MessageReader<Damage>,
    mut health: Query<(&StableEntityId, &mut RepHealth), With<Replicated>>,
) {
    for message in messages.read() {
        for (stable, mut health) in &mut health {
            if *stable == message.target {
                health.hp -= message.amount;
            }
        }
    }
}

fn spawn_projectiles(mut messages: MessageReader<SpawnProjectile>, mut commands: Commands) {
    for message in messages.read() {
        commands.spawn((
            Replicated,
            RepProjectile {
                owner: message.owner,
            },
        ));
    }
}

#[test]
fn replicated_tick_replays_messages_through_normal_bevy_systems() {
    let mut app = app();
    let target = StableEntityId::from_raw(7);
    app.world_mut()
        .spawn((target, Replicated, RepHealth { hp: 100 }));
    app.world_mut().insert_resource(RepWorldClock { hour: 13 });
    app.world_mut().save_replicated_state(98);
    app.world_mut()
        .resource_mut::<ReplicatedTimeline<Damage>>()
        .push_at(
            99,
            Damage {
                tick: 99,
                target,
                amount: 10,
            },
        );
    app.world_mut()
        .resource_mut::<ReplicatedTimeline<Damage>>()
        .push_at(
            100,
            Damage {
                tick: 100,
                target,
                amount: 20,
            },
        );

    app.world_mut().replay_replicated_ticks(98, 100).unwrap();

    let entity = entity_with_stable(app.world_mut(), target).unwrap();
    assert_eq!(
        app.world().get::<RepHealth>(entity),
        Some(&RepHealth { hp: 70 })
    );
    assert_eq!(
        app.world()
            .resource::<ReplicatedComponentHistory<RepHealth>>()
            .snapshot(100)
            .unwrap()
            .values()
            .get(&target),
        Some(&RepHealth { hp: 70 })
    );
    assert_eq!(
        app.world()
            .resource::<RollbackReplicationClock>()
            .current_tick,
        100
    );
}

#[test]
fn replicated_tick_assigns_stable_ids_before_saving_spawned_entities() {
    let mut app = app();
    let owner = StableEntityId::from_raw(30);
    app.world_mut()
        .resource_mut::<ReplicatedTimeline<SpawnProjectile>>()
        .push_at(2, SpawnProjectile { tick: 2, owner });

    app.world_mut().run_replicated_tick(2);

    let snapshot = app
        .world()
        .resource::<ReplicatedComponentHistory<RepProjectile>>()
        .snapshot(2)
        .unwrap();
    assert_eq!(snapshot.values().len(), 1);
    let (stable, projectile) = snapshot.values().first_key_value().unwrap();
    assert!(stable.is_valid());
    assert_eq!(projectile, &RepProjectile { owner });
}

#[test]
fn manual_save_assigns_stable_ids_before_capturing_replicated_state() {
    let mut app = app();
    app.world_mut().spawn((
        Replicated,
        RepProjectile {
            owner: StableEntityId::from_raw(31),
        },
    ));

    app.world_mut().save_replicated_state(3);

    let snapshot = app
        .world()
        .resource::<ReplicatedComponentHistory<RepProjectile>>()
        .snapshot(3)
        .unwrap();
    assert_eq!(snapshot.values().len(), 1);
    assert!(snapshot.values().keys().next().unwrap().is_valid());
}

#[test]
fn replicated_tick_replay_consumes_replace_for_replay_reissue_buffer() {
    let mut app = app();
    let target = StableEntityId::from_raw(32);
    app.world_mut()
        .spawn((target, Replicated, RepHealth { hp: 100 }));
    app.world_mut().save_replicated_state(1);
    app.world_mut()
        .resource_mut::<ReplicatedTimeline<Damage>>()
        .replace_for_replay([(
            2,
            Damage {
                tick: 2,
                target,
                amount: 10,
            },
        )]);

    app.world_mut().replay_replicated_ticks(1, 2).unwrap();

    assert!(
        app.world()
            .resource::<ReplicatedTimeline<Damage>>()
            .reissue
            .is_empty()
    );
}

#[test]
fn correction_replay_restores_anchor_and_reissues_later_messages() {
    let mut app = app();
    let target = StableEntityId::from_raw(8);
    app.world_mut()
        .spawn((target, Replicated, RepHealth { hp: 100 }));
    app.world_mut().save_replicated_state(98);

    app.world_mut()
        .resource_mut::<ReplicatedTimeline<Damage>>()
        .replace_for_replay([
            (
                99,
                Damage {
                    tick: 99,
                    target,
                    amount: 10,
                },
            ),
            (
                100,
                Damage {
                    tick: 100,
                    target,
                    amount: 20,
                },
            ),
        ]);
    app.world_mut().replay_replicated_ticks(98, 100).unwrap();
    let entity = entity_with_stable(app.world_mut(), target).unwrap();
    assert_eq!(
        app.world().get::<RepHealth>(entity),
        Some(&RepHealth { hp: 70 })
    );

    app.world_mut()
        .resource_mut::<ReplicatedTimeline<Damage>>()
        .replace_for_replay([
            (
                99,
                Damage {
                    tick: 99,
                    target,
                    amount: 0,
                },
            ),
            (
                100,
                Damage {
                    tick: 100,
                    target,
                    amount: 20,
                },
            ),
        ]);
    app.world_mut().replay_replicated_ticks(98, 100).unwrap();

    let entity = entity_with_stable(app.world_mut(), target).unwrap();
    assert_eq!(
        app.world().get::<RepHealth>(entity),
        Some(&RepHealth { hp: 80 })
    );
}

#[test]
fn restore_failure_does_not_partially_mutate_world() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        crate::core::AfterglowCorePlugin,
        crate::network::AfterglowNetworkPlugin,
    ));
    let stable = StableEntityId::from_raw(40);
    let entity = app
        .world_mut()
        .spawn((stable, Replicated, RepHealth { hp: 100 }))
        .id();
    app.replicate(component::<RepHealth>());
    app.world_mut().save_replicated_state(1);

    app.world_mut()
        .entity_mut(entity)
        .insert(RepHealth { hp: 45 });
    app.replicate(resource::<RepWorldClock>());

    assert_eq!(
        app.world_mut().restore_replicated_state(1),
        Err(ReplicatedRollbackError::MissingSnapshot { tick: 1 })
    );
    assert_eq!(
        app.world().get::<RepHealth>(entity),
        Some(&RepHealth { hp: 45 })
    );
    assert!(app.world().get_entity(entity).is_ok());
}

#[test]
fn restore_rebuilds_replicated_components_resources_and_missing_entities() {
    let mut app = app();
    let original = StableEntityId::from_raw(10);
    let stale = StableEntityId::from_raw(11);
    let original_entity = app
        .world_mut()
        .spawn((original, Replicated, RepHealth { hp: 100 }))
        .id();
    let stale_entity = app
        .world_mut()
        .spawn((stale, Replicated, RepHealth { hp: 1 }))
        .id();
    app.world_mut().insert_resource(RepWorldClock { hour: 5 });
    app.world_mut().save_replicated_state(50);

    app.world_mut().despawn(original_entity);
    app.world_mut()
        .entity_mut(stale_entity)
        .insert(RepHealth { hp: 99 });
    app.world_mut().insert_resource(RepWorldClock { hour: 23 });

    app.world_mut().restore_replicated_state(50).unwrap();

    let restored_entity = entity_with_stable(app.world_mut(), original).unwrap();
    assert_eq!(
        app.world().get::<RepHealth>(restored_entity),
        Some(&RepHealth { hp: 100 })
    );
    assert_eq!(
        app.world().get::<RepHealth>(stale_entity),
        Some(&RepHealth { hp: 1 })
    );
    assert_eq!(
        app.world().resource::<RepWorldClock>(),
        &RepWorldClock { hour: 5 }
    );
}

#[test]
fn restore_removes_absent_resource_snapshot() {
    let mut app = app();
    app.world_mut().save_replicated_state(1);
    app.world_mut().insert_resource(RepWorldClock { hour: 23 });

    app.world_mut().restore_replicated_state(1).unwrap();

    assert!(app.world().get_resource::<RepWorldClock>().is_none());
    assert_eq!(
        app.world()
            .resource::<ReplicatedResourceState<RepWorldClock>>()
            .get(),
        None
    );
}

#[test]
fn restore_keeps_entity_when_another_replicated_component_remains() {
    let mut app = app();
    let stable = StableEntityId::from_raw(41);
    let entity = app
        .world_mut()
        .spawn((stable, Replicated, RepMana { mp: 5 }))
        .id();
    app.world_mut().save_replicated_state(1);
    app.world_mut()
        .entity_mut(entity)
        .insert(RepHealth { hp: 20 });

    app.world_mut().restore_replicated_state(1).unwrap();

    assert!(app.world().get_entity(entity).is_ok());
    assert_eq!(app.world().get::<RepHealth>(entity), None);
    assert_eq!(app.world().get::<RepMana>(entity), Some(&RepMana { mp: 5 }));
}

#[test]
fn restore_despawns_entities_absent_from_replicated_anchor_snapshot() {
    let mut app = app();
    let kept = StableEntityId::from_raw(12);
    let removed = StableEntityId::from_raw(13);
    app.world_mut()
        .spawn((kept, Replicated, RepHealth { hp: 100 }));
    app.world_mut().save_replicated_state(1);
    let removed_entity = app
        .world_mut()
        .spawn((removed, Replicated, RepHealth { hp: 20 }))
        .id();

    app.world_mut().restore_replicated_state(1).unwrap();

    assert_eq!(app.world().get::<RepHealth>(removed_entity), None);
    assert!(app.world().get_entity(removed_entity).is_err());
    assert_eq!(
        app.world()
            .resource::<ReplicatedComponentState<RepHealth>>()
            .values()
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        [kept]
    );
}

#[test]
fn replay_reports_missing_snapshots_and_invalid_ranges() {
    let mut app = app();
    assert_eq!(
        app.world_mut().restore_replicated_state(404),
        Err(ReplicatedRollbackError::MissingSnapshot { tick: 404 })
    );
    app.world_mut().save_replicated_state(10);
    assert_eq!(
        app.world_mut().replay_replicated_ticks(10, 9),
        Err(ReplicatedRollbackError::InvalidRange {
            anchor_tick: 10,
            through_tick: 9
        })
    );
}

#[test]
fn histories_prune_by_rollback_policy_window() {
    let mut app = app();
    app.world_mut()
        .resource_mut::<RollbackReplicationClock>()
        .policy = RollbackPolicy {
        max_rollback_ticks: 2,
        commit_delay_ticks: 1,
    };
    let target = StableEntityId::from_raw(20);
    app.world_mut()
        .spawn((target, Replicated, RepHealth { hp: 100 }));

    for tick in 10..=13 {
        app.world_mut().save_replicated_state(tick);
    }

    let history = app
        .world()
        .resource::<ReplicatedComponentHistory<RepHealth>>();
    assert!(history.snapshot(10).is_none());
    assert!(history.snapshot(11).is_some());
    assert!(history.snapshot(13).is_some());
}

fn entity_with_stable(world: &mut World, stable: StableEntityId) -> Option<Entity> {
    let mut query = world.query::<(Entity, &StableEntityId)>();
    query
        .iter(world)
        .find_map(|(entity, id)| (*id == stable).then_some(entity))
}
