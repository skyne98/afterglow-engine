use avian3d::prelude::{LinearVelocity, RigidBody};
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::{Interpolated, Predicted, Rollback};

use super::*;
use crate::{
    demos::multiplayer_boxes::scene::attach_predicted_player_physics,
    input::{AfterglowAction, default_gameplay_input_map},
    network::{AfterglowNetworkContext, LightyearRole},
};

#[test]
fn collect_input_recovers_after_action_state_was_restored_to_zero() {
    let mut app = test_app();
    app.init_resource::<ButtonInput<KeyCode>>();
    app.add_systems(Update, collect_input);

    let entity = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "2".to_string(),
            },
            Predicted,
            default_gameplay_input_map(),
            ActionState::<AfterglowAction>::default(),
        ))
        .id();

    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyW);
    app.update();
    assert_move_axis(&app, entity, Vec2::Y);

    app.world_mut()
        .entity_mut(entity)
        .get_mut::<ActionState<AfterglowAction>>()
        .unwrap()
        .set_axis_pair(&AfterglowAction::Move, Vec2::ZERO);
    app.update();

    assert_move_axis(&app, entity, Vec2::Y);
}

#[test]
fn predicted_physics_is_attached_before_fixed_movement() {
    let mut app = test_app();
    app.insert_resource(AfterglowNetworkContext::new(LightyearRole::Client, 2));
    app.world_mut().resource_mut::<DemoInput>().0 = Vec2::Y;
    let mut action_state = ActionState::<AfterglowAction>::default();
    action_state.set_axis_pair(&AfterglowAction::Move, Vec2::Y);
    let player = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "2".to_string(),
            },
            Predicted,
            Transform::from_xyz(0.0, 0.4, 0.0),
            action_state,
        ))
        .id();

    app.add_systems(PreUpdate, attach_predicted_player_physics);
    app.add_systems(Update, apply_movement);
    app.update();

    assert!(app.world().get::<RigidBody>(player).is_some());
    let velocity = app.world().get::<LinearVelocity>(player).unwrap().0;
    assert!(
        velocity.z > 0.0,
        "movement should use LinearVelocity on the first fixed tick, not direct Transform fallback"
    );
}

#[test]
fn local_prediction_uses_action_state_for_movement() {
    let mut app = test_app();
    app.insert_resource(AfterglowNetworkContext::new(LightyearRole::Client, 2));
    let mut action_state = ActionState::<AfterglowAction>::default();
    action_state.set_axis_pair(&AfterglowAction::Move, Vec2::Y);
    let player = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "2".to_string(),
            },
            Predicted,
            LinearVelocity::ZERO,
            action_state,
        ))
        .id();

    app.add_systems(Update, apply_movement);
    app.update();

    let velocity = app.world().get::<LinearVelocity>(player).unwrap().0;
    assert!(
        velocity.z > 0.0,
        "local predicted player should move from ActionState: velocity={velocity:?}"
    );
}

#[test]
fn collect_input_is_guarded_during_rollback() {
    let mut app = test_app();
    app.insert_resource(AfterglowNetworkContext::new(LightyearRole::Client, 2));
    app.init_resource::<ButtonInput<KeyCode>>();
    app.world_mut().resource_mut::<DemoInput>().0 = Vec2::Y;
    app.world_mut().spawn(Rollback::FromState);

    let player = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "2".to_string(),
            },
            Predicted,
            default_gameplay_input_map(),
            ActionState::<AfterglowAction>::default(),
        ))
        .id();

    app.add_systems(Update, collect_input);
    app.update();

    assert_move_axis(&app, player, Vec2::ZERO);
}

#[test]
fn remote_predicted_players_use_their_own_rebroadcast_action_state() {
    let mut app = test_app();
    app.insert_resource(AfterglowNetworkContext::new(LightyearRole::Client, 2));
    app.world_mut().resource_mut::<DemoInput>().0 = Vec2::ZERO;

    let mut action_state = ActionState::<AfterglowAction>::default();
    action_state.set_axis_pair(&AfterglowAction::Move, Vec2::Y);
    let remote = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "3".to_string(),
            },
            Predicted,
            LinearVelocity::ZERO,
            action_state,
        ))
        .id();

    app.add_systems(Update, apply_movement);
    app.update();

    assert!(
        app.world().get::<LinearVelocity>(remote).unwrap().0.z > 0.0,
        "remote predicted players should move from rebroadcast ActionState for local contact prediction"
    );
}

#[test]
fn client_predicted_movement_skips_interpolated_players() {
    let mut app = test_app();
    let mut action_state = ActionState::<AfterglowAction>::default();
    action_state.set_axis_pair(&AfterglowAction::Move, Vec2::Y);
    let entity = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "3".to_string(),
            },
            Interpolated,
            LinearVelocity::ZERO,
            action_state,
        ))
        .id();

    app.add_systems(Update, apply_predicted_movement);
    app.update();

    assert_eq!(
        app.world().get::<LinearVelocity>(entity).unwrap().0,
        Vec3::ZERO,
        "client movement must not fight interpolation on non-predicted players"
    );
}

#[test]
fn non_player_predicted_entities_are_not_moved_by_player_input_system() {
    let mut app = test_app();
    app.insert_resource(AfterglowNetworkContext::new(LightyearRole::Client, 2));

    let mut action_state = ActionState::<AfterglowAction>::default();
    action_state.set_axis_pair(&AfterglowAction::Move, Vec2::Y);
    let entity = app
        .world_mut()
        .spawn((Predicted, LinearVelocity::ZERO, action_state))
        .id();

    app.add_systems(Update, apply_movement);
    app.update();

    assert_eq!(
        app.world().get::<LinearVelocity>(entity).unwrap().0,
        Vec3::ZERO,
        "apply_movement must be scoped to PlayerBox entities"
    );
}

#[test]
fn remote_players_do_not_use_local_immediate_input_fallback() {
    let mut app = test_app();
    app.insert_resource(AfterglowNetworkContext::new(LightyearRole::Client, 2));
    app.world_mut().resource_mut::<DemoInput>().0 = Vec2::Y;

    let remote = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "3".to_string(),
            },
            Predicted,
            LinearVelocity::ZERO,
            ActionState::<AfterglowAction>::default(),
        ))
        .id();

    app.add_systems(Update, apply_movement);
    app.update();

    assert_eq!(
        app.world().get::<LinearVelocity>(remote).unwrap().0,
        Vec3::ZERO
    );
}

#[test]
fn rope_release_during_rollback_is_replayed_after_rollback() {
    let mut app = test_app();
    app.init_resource::<ButtonInput<KeyCode>>();
    app.add_systems(Update, collect_input);

    let rollback = app.world_mut().spawn(Rollback::FromState).id();
    let player = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "2".to_string(),
            },
            Predicted,
            default_gameplay_input_map(),
            ActionState::<AfterglowAction>::default(),
        ))
        .id();

    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyF);
    app.update();
    assert!(
        !app.world()
            .get::<ActionState<AfterglowAction>>(player)
            .unwrap()
            .pressed(&AfterglowAction::RopeToggle),
        "rollback guard should still prevent direct ActionState writes"
    );

    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyF);
    app.update();
    assert!(
        !app.world()
            .get::<ActionState<AfterglowAction>>(player)
            .unwrap()
            .just_released(&AfterglowAction::RopeToggle),
        "release edge should be buffered, not written during rollback"
    );

    app.world_mut().entity_mut(rollback).despawn();
    app.update();
    assert!(
        app.world()
            .get::<ActionState<AfterglowAction>>(player)
            .unwrap()
            .just_released(&AfterglowAction::RopeToggle),
        "physical rope release observed during rollback must be replayed once input writing resumes"
    );
}

fn assert_move_axis(app: &App, entity: Entity, expected: Vec2) {
    let axis = app
        .world()
        .get::<ActionState<AfterglowAction>>(entity)
        .unwrap()
        .clamped_axis_pair(&AfterglowAction::Move);
    assert_eq!(axis, expected);
}
