use super::*;
use crate::core::identity::{Replicated, StableEntityId};
use bevy::prelude::*;

#[derive(Component, Clone, Debug, Eq, PartialEq)]
struct EdgeHealth {
    hp: i32,
}

impl Replicate for EdgeHealth {
    const REPLICATION_NAME: &'static str = "tests::EdgeHealth";
}

fn replication_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        crate::core::AfterglowCorePlugin,
        crate::network::AfterglowNetworkPlugin,
    ));
    app.replicate(component::<EdgeHealth>());
    app
}

#[test]
fn same_frame_replicated_marker_remove_and_readd_keeps_component_state() {
    let mut app = replication_app();
    let stable = StableEntityId::from_raw(811);
    let entity = app
        .world_mut()
        .spawn((stable, Replicated, EdgeHealth { hp: 91 }))
        .id();
    app.update();

    app.world_mut().entity_mut(entity).remove::<Replicated>();
    app.world_mut().entity_mut(entity).insert(Replicated);
    app.update();

    let state = app
        .world()
        .resource::<ReplicatedComponentState<EdgeHealth>>();
    assert_eq!(state.get(stable), Some(&EdgeHealth { hp: 91 }));
    assert!(!state.removed().contains(&stable));
}

#[test]
fn same_frame_component_remove_and_readd_keeps_component_state() {
    let mut app = replication_app();
    let stable = StableEntityId::from_raw(812);
    let entity = app
        .world_mut()
        .spawn((stable, Replicated, EdgeHealth { hp: 92 }))
        .id();
    app.update();

    app.world_mut().entity_mut(entity).remove::<EdgeHealth>();
    app.world_mut()
        .entity_mut(entity)
        .insert(EdgeHealth { hp: 93 });
    app.update();

    let state = app
        .world()
        .resource::<ReplicatedComponentState<EdgeHealth>>();
    assert_eq!(state.get(stable), Some(&EdgeHealth { hp: 93 }));
    assert!(!state.removed().contains(&stable));
}

#[test]
fn invalid_stable_ids_are_not_collected_as_replicated_state() {
    let mut app = replication_app();
    let entity = app
        .world_mut()
        .spawn((StableEntityId::INVALID, Replicated, EdgeHealth { hp: 94 }))
        .id();

    app.update();

    let state = app
        .world()
        .resource::<ReplicatedComponentState<EdgeHealth>>();
    assert_eq!(state.get(StableEntityId::INVALID), None);

    app.update();

    let stable = app.world().get::<StableEntityId>(entity).copied().unwrap();
    let state = app
        .world()
        .resource::<ReplicatedComponentState<EdgeHealth>>();
    assert!(stable.is_valid());
    assert_eq!(state.get(stable), Some(&EdgeHealth { hp: 94 }));
}

#[test]
fn changing_to_invalid_stable_id_removes_previous_replicated_state() {
    let mut app = replication_app();
    let stable = StableEntityId::from_raw(813);
    let entity = app
        .world_mut()
        .spawn((stable, Replicated, EdgeHealth { hp: 95 }))
        .id();
    app.update();

    app.world_mut()
        .entity_mut(entity)
        .insert(StableEntityId::INVALID);
    app.update();

    let state = app
        .world()
        .resource::<ReplicatedComponentState<EdgeHealth>>();
    assert_eq!(state.get(stable), None);
    assert!(state.removed().contains(&stable));
    assert_eq!(state.get(StableEntityId::INVALID), None);
}
