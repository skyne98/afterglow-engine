use std::time::Duration;

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::{PreSpawned, Predicted, PredictionDisable, Rollback};

use super::*;
use crate::{
    core::identity::StableEntityId,
    input::{AfterglowAction, default_gameplay_input_map},
    network::connection::{ClientSpawned, LocalPlayerId},
};

fn rope_hider_test_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::input::InputPlugin,
        lightyear::prelude::server::ServerPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / 60.0),
        },
        leafwing_input_manager::plugin::InputManagerPlugin::<AfterglowAction>::default(),
    ));
    super::super::network::register_demo_protocol(&mut app);
    app.insert_resource(LocalPlayerId(2));
    app
}

fn run_rope_frame(app: &mut App) {
    app.world_mut().run_schedule(FixedUpdate);
}

fn release_rope_toggle(app: &mut App) {
    let mut world = app.world_mut();
    let mut query = world.query::<&mut ActionState<AfterglowAction>>();
    for mut action in query.iter_mut(&mut world) {
        action.press(&AfterglowAction::RopeToggle);
        action.release(&AfterglowAction::RopeToggle);
    }
}

fn test_box_id(index: u32) -> StableEntityId {
    StableEntityId::new(10_000 + u128::from(index))
}

fn active_visible_rope_count(app: &mut App, owner: &str) -> usize {
    app.world_mut()
        .query::<(
            &RopeLink,
            Option<&super::super::rope::RopeJointDetachPending>,
            Option<&PredictionDisable>,
        )>()
        .iter(app.world())
        .filter(|(link, detach_pending, disabled)| {
            link.player_owner == owner && detach_pending.is_none() && disabled.is_none()
        })
        .count()
}

fn spawn_predicted_local_player_and_box(app: &mut App) {
    app.world_mut().spawn((
        PlayerBox { owner: "2".into() },
        Transform::from_xyz(0.0, 0.4, 0.0),
        avian3d::prelude::RigidBody::Dynamic,
        LinearVelocity::ZERO,
        default_gameplay_input_map(),
        ActionState::<AfterglowAction>::default(),
        Predicted,
    ));
    app.world_mut().spawn((
        KinematicBox {
            initial_pos: Vec3::new(1.0, 0.5, 0.0),
        },
        test_box_id(0),
        Transform::from_xyz(1.0, 0.5, 0.0),
        avian3d::prelude::RigidBody::Dynamic,
        LinearVelocity::ZERO,
    ));
}

fn spawn_client_rope(app: &mut App, client_link: Entity) -> Entity {
    let target = test_box_id(0);
    let rope_id = super::super::rope::rope_id_for_input("2", target);
    app.world_mut()
        .spawn((
            RopeLink {
                rope_id,
                player_owner: "2".to_string(),
                target,
            },
            rope_id,
            PreSpawned::new(super::super::rope::rope_link_hash(rope_id)).for_receiver(client_link),
        ))
        .id()
}

#[test]
fn initial_attach_tap_is_not_suppressed_by_physical_release_hider() {
    let mut app = rope_hider_test_app();
    app.init_resource::<super::super::rope::LocallyReleasedRopes>();
    app.world_mut().spawn(ClientSpawned);
    spawn_predicted_local_player_and_box(&mut app);
    app.add_systems(
        FixedUpdate,
        (
            super::super::rope::toggle_rope,
            super::super::rope::hide_local_rope_on_physical_release,
        )
            .chain(),
    );

    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyF);
    {
        let mut world = app.world_mut();
        let mut query = world.query::<&mut ActionState<AfterglowAction>>();
        for mut action in query.iter_mut(&mut world) {
            action.press(&AfterglowAction::RopeToggle);
        }
    }
    run_rope_frame(&mut app);

    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyF);
    release_rope_toggle(&mut app);
    run_rope_frame(&mut app);

    assert_eq!(active_visible_rope_count(&mut app, "2"), 1);
}

#[test]
fn physical_release_hider_suppresses_without_action_state_edge() {
    let mut app = rope_hider_test_app();
    app.init_resource::<super::super::rope::LocallyReleasedRopes>();
    let client_link = app.world_mut().spawn(ClientSpawned).id();
    spawn_predicted_local_player_and_box(&mut app);
    let rope = spawn_client_rope(&mut app, client_link);
    let rope_id = *app.world().get::<StableEntityId>(rope).unwrap();
    app.add_systems(
        FixedUpdate,
        super::super::rope::hide_local_rope_on_physical_release,
    );

    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyF);
    run_rope_frame(&mut app);
    assert_eq!(active_visible_rope_count(&mut app, "2"), 1);

    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyF);
    run_rope_frame(&mut app);

    assert_eq!(active_visible_rope_count(&mut app, "2"), 0);
    assert!(app.world().get::<PredictionDisable>(rope).is_some());
    assert!(
        app.world()
            .resource::<super::super::rope::LocallyReleasedRopes>()
            .contains(rope_id)
    );
}

#[test]
fn physical_release_hider_skips_rollback_and_hides_afterward() {
    let mut app = rope_hider_test_app();
    app.init_resource::<super::super::rope::LocallyReleasedRopes>();
    let client_link = app.world_mut().spawn(ClientSpawned).id();
    spawn_predicted_local_player_and_box(&mut app);
    let rope = spawn_client_rope(&mut app, client_link);
    let rope_id = *app.world().get::<StableEntityId>(rope).unwrap();
    app.add_systems(
        FixedUpdate,
        super::super::rope::hide_local_rope_on_physical_release,
    );

    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyF);
    run_rope_frame(&mut app);
    let rollback = app.world_mut().spawn(Rollback::FromState).id();

    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyF);
    run_rope_frame(&mut app);
    assert_eq!(active_visible_rope_count(&mut app, "2"), 1);
    assert!(
        !app.world()
            .resource::<super::super::rope::LocallyReleasedRopes>()
            .contains(rope_id)
    );

    app.world_mut().entity_mut(rollback).remove::<Rollback>();
    run_rope_frame(&mut app);
    assert_eq!(active_visible_rope_count(&mut app, "2"), 0);
    assert!(app.world().get::<PredictionDisable>(rope).is_some());
    assert!(
        app.world()
            .resource::<super::super::rope::LocallyReleasedRopes>()
            .contains(rope_id)
    );
}
