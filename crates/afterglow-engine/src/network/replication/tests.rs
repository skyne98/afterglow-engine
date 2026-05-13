use super::*;
use crate::core::identity::{Persistent, Replicated, StableEntityId};
use bevy::prelude::*;

#[derive(Component, Clone, Debug, Eq, PartialEq)]
struct RepHealth {
    hp: i32,
}

impl Replicate for RepHealth {
    const REPLICATION_NAME: &'static str = "tests::RepHealth";
}

#[derive(Resource, Clone, Debug, Eq, PartialEq)]
struct RepDungeonClock {
    hour: u8,
}

impl Replicate for RepDungeonClock {
    const REPLICATION_NAME: &'static str = "tests::RepDungeonClock";
}

#[derive(Message, Clone, Debug, Eq, PartialEq)]
struct OpenDoorCommand {
    entity: StableEntityId,
}

impl ReplicatedCommand for OpenDoorCommand {
    fn tick(&self) -> u32 {
        10
    }
}

#[derive(Message, Clone, Debug, Eq, PartialEq)]
struct DamageApplied {
    entity: StableEntityId,
    amount: i32,
}

impl ReplicationEvent for DamageApplied {
    fn tick(&self) -> u32 {
        10
    }
}

fn emit_timeline(
    mut commands: MessageWriter<OpenDoorCommand>,
    mut events: MessageWriter<DamageApplied>,
) {
    let entity = StableEntityId::from_raw(7);
    commands.write(OpenDoorCommand { entity });
    events.write(DamageApplied { entity, amount: 25 });
}

#[derive(Resource, Default)]
struct SeenDamage(Vec<DamageApplied>);

fn read_damage(mut events: MessageReader<DamageApplied>, mut seen: ResMut<SeenDamage>) {
    seen.0.extend(events.read().cloned());
}

#[test]
fn app_replicate_registers_components_resources_commands_and_events() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        crate::core::AfterglowCorePlugin,
        crate::network::AfterglowNetworkPlugin,
    ));
    let entity = StableEntityId::from_raw(7);
    app.insert_resource(RepDungeonClock { hour: 13 });
    app.world_mut()
        .spawn((entity, RepHealth { hp: 100 }, Replicated));
    app.replicate(component::<RepHealth>())
        .replicate(resource::<RepDungeonClock>())
        .replicate(command::<OpenDoorCommand>())
        .replicate(event::<DamageApplied>())
        .add_systems(Update, emit_timeline.before(ReplicationSet::CollectChanges));

    app.update();

    let health = app
        .world()
        .resource::<ReplicatedComponentState<RepHealth>>();
    assert_eq!(health.get(entity), Some(&RepHealth { hp: 100 }));
    let clock = app
        .world()
        .resource::<ReplicatedResourceState<RepDungeonClock>>();
    assert_eq!(clock.get(), Some(&RepDungeonClock { hour: 13 }));
    assert_eq!(
        app.world()
            .resource::<ReplicatedTimeline<OpenDoorCommand>>()
            .messages_at(10),
        &[OpenDoorCommand { entity }]
    );
    assert_eq!(
        app.world()
            .resource::<ReplicatedTimeline<DamageApplied>>()
            .messages_at(10),
        &[DamageApplied { entity, amount: 25 }]
    );

    let registry = app.world().resource::<ReplicationRegistry>();
    assert!(registry.components.contains("tests::RepHealth"));
    assert!(registry.resources.contains("tests::RepDungeonClock"));
}

#[test]
fn duplicate_timeline_registration_collects_messages_once() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        crate::core::AfterglowCorePlugin,
        crate::network::AfterglowNetworkPlugin,
    ));
    let entity = StableEntityId::from_raw(7);
    app.replicate(command::<OpenDoorCommand>())
        .replicate(command::<OpenDoorCommand>())
        .replicate(event::<DamageApplied>())
        .replicate(event::<DamageApplied>())
        .add_systems(Update, emit_timeline.before(ReplicationSet::CollectChanges));

    app.update();

    assert_eq!(
        app.world()
            .resource::<ReplicatedTimeline<OpenDoorCommand>>()
            .messages_at(10),
        &[OpenDoorCommand { entity }]
    );
    assert_eq!(
        app.world()
            .resource::<ReplicatedTimeline<DamageApplied>>()
            .messages_at(10),
        &[DamageApplied { entity, amount: 25 }]
    );
}

#[test]
fn component_removal_is_tracked_as_removed_replicated_state() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        crate::core::AfterglowCorePlugin,
        crate::network::AfterglowNetworkPlugin,
    ));
    let stable = StableEntityId::from_raw(77);
    let entity = app
        .world_mut()
        .spawn((stable, RepHealth { hp: 50 }, Replicated))
        .id();
    app.replicate(component::<RepHealth>());

    app.update();
    assert_eq!(
        app.world()
            .resource::<ReplicatedComponentState<RepHealth>>()
            .get(stable),
        Some(&RepHealth { hp: 50 })
    );

    app.world_mut().entity_mut(entity).remove::<RepHealth>();
    app.update();

    let state = app
        .world()
        .resource::<ReplicatedComponentState<RepHealth>>();
    assert_eq!(state.get(stable), None);
    assert!(state.removed().contains(&stable));
}

#[test]
fn replicated_component_is_collected_after_stable_id_assignment() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        crate::core::AfterglowCorePlugin,
        crate::network::AfterglowNetworkPlugin,
    ));
    let entity = app
        .world_mut()
        .spawn((RepHealth { hp: 75 }, Replicated))
        .id();
    app.replicate(component::<RepHealth>());

    app.update();
    let stable = app.world().get::<StableEntityId>(entity).copied().unwrap();
    assert!(
        app.world()
            .resource::<ReplicatedComponentState<RepHealth>>()
            .get(stable)
            .is_none()
    );

    app.update();

    assert_eq!(
        app.world()
            .resource::<ReplicatedComponentState<RepHealth>>()
            .get(stable),
        Some(&RepHealth { hp: 75 })
    );
}

#[test]
fn stable_id_change_rekeys_replicated_component_state() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        crate::core::AfterglowCorePlugin,
        crate::network::AfterglowNetworkPlugin,
    ));
    let first = StableEntityId::from_raw(501);
    let second = StableEntityId::from_raw(502);
    let entity = app
        .world_mut()
        .spawn((first, RepHealth { hp: 33 }, Replicated))
        .id();
    app.replicate(component::<RepHealth>());

    app.update();
    assert_eq!(
        app.world()
            .resource::<ReplicatedComponentState<RepHealth>>()
            .get(first),
        Some(&RepHealth { hp: 33 })
    );

    app.world_mut().entity_mut(entity).insert(second);
    app.update();

    let state = app
        .world()
        .resource::<ReplicatedComponentState<RepHealth>>();
    assert_eq!(state.get(first), None);
    assert!(state.removed().contains(&first));
    assert_eq!(state.get(second), Some(&RepHealth { hp: 33 }));
}

#[test]
fn stable_id_swap_keeps_both_replicated_component_values() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        crate::core::AfterglowCorePlugin,
        crate::network::AfterglowNetworkPlugin,
    ));
    let first = StableEntityId::from_raw(601);
    let second = StableEntityId::from_raw(602);
    let first_entity = app
        .world_mut()
        .spawn((first, RepHealth { hp: 10 }, Replicated))
        .id();
    let second_entity = app
        .world_mut()
        .spawn((second, RepHealth { hp: 20 }, Replicated))
        .id();
    app.replicate(component::<RepHealth>());

    app.update();
    app.world_mut().entity_mut(first_entity).insert(second);
    app.world_mut().entity_mut(second_entity).insert(first);
    app.update();

    let state = app
        .world()
        .resource::<ReplicatedComponentState<RepHealth>>();
    assert_eq!(state.get(first), Some(&RepHealth { hp: 20 }));
    assert_eq!(state.get(second), Some(&RepHealth { hp: 10 }));
}

#[test]
fn persistent_only_components_are_not_collected_as_replicated_state() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        crate::core::AfterglowCorePlugin,
        crate::network::AfterglowNetworkPlugin,
    ));
    let stable = StableEntityId::from_raw(701);
    app.world_mut()
        .spawn((stable, Persistent, RepHealth { hp: 44 }));
    app.replicate(component::<RepHealth>());

    app.update();

    let state = app
        .world()
        .resource::<ReplicatedComponentState<RepHealth>>();
    assert_eq!(state.get(stable), None);
    assert!(!state.removed().contains(&stable));
}

#[test]
fn adding_replicated_marker_collects_existing_component_state() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        crate::core::AfterglowCorePlugin,
        crate::network::AfterglowNetworkPlugin,
    ));
    let stable = StableEntityId::from_raw(702);
    let entity = app
        .world_mut()
        .spawn((stable, Persistent, RepHealth { hp: 45 }))
        .id();
    app.replicate(component::<RepHealth>());

    app.update();
    app.world_mut().entity_mut(entity).insert(Replicated);
    app.update();

    assert_eq!(
        app.world()
            .resource::<ReplicatedComponentState<RepHealth>>()
            .get(stable),
        Some(&RepHealth { hp: 45 })
    );
}

#[test]
fn removing_replicated_marker_removes_component_state() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        crate::core::AfterglowCorePlugin,
        crate::network::AfterglowNetworkPlugin,
    ));
    let stable = StableEntityId::from_raw(703);
    let entity = app
        .world_mut()
        .spawn((stable, Replicated, RepHealth { hp: 46 }))
        .id();
    app.replicate(component::<RepHealth>());

    app.update();
    app.world_mut().entity_mut(entity).remove::<Replicated>();
    app.update();

    let state = app
        .world()
        .resource::<ReplicatedComponentState<RepHealth>>();
    assert_eq!(state.get(stable), None);
    assert!(state.removed().contains(&stable));
}

#[test]
fn resource_removal_clears_replicated_resource_state() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        crate::core::AfterglowCorePlugin,
        crate::network::AfterglowNetworkPlugin,
    ));
    app.insert_resource(RepDungeonClock { hour: 13 });
    app.replicate(resource::<RepDungeonClock>());

    app.update();
    assert_eq!(
        app.world()
            .resource::<ReplicatedResourceState<RepDungeonClock>>()
            .get(),
        Some(&RepDungeonClock { hour: 13 })
    );

    app.world_mut().remove_resource::<RepDungeonClock>();
    app.update();

    assert_eq!(
        app.world()
            .resource::<ReplicatedResourceState<RepDungeonClock>>()
            .get(),
        None
    );
}

#[test]
fn rollback_replay_reissues_relevant_timeline_messages_to_bevy_readers() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        crate::core::AfterglowCorePlugin,
        crate::network::AfterglowNetworkPlugin,
    ));
    let entity = StableEntityId::from_raw(7);
    app.init_resource::<SeenDamage>()
        .replicate(event::<DamageApplied>())
        .add_systems(
            Update,
            read_damage
                .after(ReplicationSet::ReissueMessages)
                .before(ReplicationSet::CollectChanges),
        );

    app.world_mut()
        .resource_mut::<ReplicatedTimeline<DamageApplied>>()
        .replace_for_replay([(
            10,
            DamageApplied {
                entity,
                amount: 100,
            },
        )]);
    app.update();

    assert_eq!(
        app.world().resource::<SeenDamage>().0,
        [DamageApplied {
            entity,
            amount: 100
        }]
    );
    assert_eq!(
        app.world()
            .resource::<ReplicatedTimeline<DamageApplied>>()
            .messages_at(10),
        &[DamageApplied {
            entity,
            amount: 100
        }]
    );
}

#[test]
fn replay_collection_does_not_drop_unread_real_messages() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        crate::core::AfterglowCorePlugin,
        crate::network::AfterglowNetworkPlugin,
    ));
    let entity = StableEntityId::from_raw(7);
    app.replicate(event::<DamageApplied>());

    app.world_mut()
        .resource_mut::<Messages<DamageApplied>>()
        .write(DamageApplied { entity, amount: 5 });
    app.world_mut()
        .resource_mut::<ReplicatedTimeline<DamageApplied>>()
        .replace_for_replay([(
            10,
            DamageApplied {
                entity,
                amount: 100,
            },
        )]);
    app.update();

    assert_eq!(
        app.world()
            .resource::<ReplicatedTimeline<DamageApplied>>()
            .messages_at(10),
        &[
            DamageApplied {
                entity,
                amount: 100
            },
            DamageApplied { entity, amount: 5 }
        ]
    );
}

#[test]
fn timeline_rejects_out_of_order_messages_older_than_retention_window() {
    let entity = StableEntityId::from_raw(7);
    let mut timeline = ReplicatedTimeline::<DamageApplied>::default();

    timeline.push_at(200, DamageApplied { entity, amount: 1 });
    timeline.push_at(79, DamageApplied { entity, amount: 2 });
    timeline.push_at(80, DamageApplied { entity, amount: 3 });

    assert!(timeline.messages_at(79).is_empty());
    assert_eq!(
        timeline.messages_at(80),
        &[DamageApplied { entity, amount: 3 }]
    );
    assert_eq!(
        timeline.messages_at(200),
        &[DamageApplied { entity, amount: 1 }]
    );
}
