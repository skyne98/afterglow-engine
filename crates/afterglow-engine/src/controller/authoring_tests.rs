use super::*;
use crate::{
    core::AfterglowCorePlugin,
    input::AfterglowAction,
    physics::{AfterglowPhysicsPlugin, PhysicsBody, PhysicsCollider},
};
use avian3d::prelude::{Collider, RigidBody};
use bevy::time::TimeUpdateStrategy;
use leafwing_input_manager::action_state::ActionState;
use std::time::Duration;

fn command(axes: &[(&str, f32)], actions: &[AfterglowAction]) -> ActionState<AfterglowAction> {
    test_input::command(axes, actions)
}

#[test]
fn plugin_authors_kinematic_cylinder_for_controller() {
    let mut app = controller_app();
    let entity = app
        .world_mut()
        .spawn((
            FirstPersonController::new(),
            Transform::from_xyz(0.0, 1.0, 0.0),
        ))
        .id();
    let crouched = app
        .world_mut()
        .spawn((
            FirstPersonController::new(),
            FirstPersonMotorState {
                stance: ControllerStance::Crouching,
                desired_stance: ControllerStance::Crouching,
                ..default()
            },
            Transform::from_xyz(0.0, 1.0, 0.0),
        ))
        .id();
    test_input::set_input(&mut app, crouched, command(&[], &[AfterglowAction::Crouch]));

    app.update();

    assert!(app.world().get::<FirstPersonMotorState>(entity).is_some());
    assert_eq!(
        app.world().get::<PhysicsBody>(entity),
        Some(&PhysicsBody::kinematic())
    );
    assert!(app.world().get::<RigidBody>(entity).is_none());
    assert!(app.world().get::<Collider>(entity).is_none());
    assert_eq!(
        app.world().get::<PhysicsCollider>(entity),
        Some(&PhysicsCollider::cylinder(
            FirstPersonControllerConfig::default().body_radius,
            FirstPersonControllerConfig::default().standing_height
        ))
    );
    assert_eq!(
        app.world().get::<PhysicsCollider>(crouched),
        Some(&PhysicsCollider::cylinder(
            FirstPersonControllerConfig::default().body_radius,
            FirstPersonControllerConfig::default().crouching_height
        ))
    );

    app.update();

    assert_eq!(
        app.world().get::<RigidBody>(entity),
        Some(&RigidBody::Kinematic)
    );
    assert!(app.world().get::<Collider>(entity).is_some());
}

#[test]
fn plugin_rejects_uncrouch_when_standing_body_does_not_fit() {
    let mut app = controller_app();
    let config = FirstPersonControllerConfig::default();
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            FirstPersonMotorState {
                stance: ControllerStance::Crouching,
                desired_stance: ControllerStance::Crouching,
                ..default()
            },
            Transform::from_xyz(0.0, config.height(ControllerStance::Crouching) * 0.5, 0.0),
        ))
        .id();
    app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(2.0, 0.2, 2.0),
        Transform::from_xyz(0.0, 1.45, 0.0),
    ));
    app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(8.0, 0.1, 8.0),
        Transform::from_xyz(0.0, -0.05, 0.0),
    ));

    test_input::set_input(&mut app, player, command(&[], &[AfterglowAction::Crouch]));
    app.update();
    test_input::set_input(&mut app, player, command(&[("move.y", 1.0)], &[]));
    app.update();

    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    assert_eq!(state.stance, ControllerStance::Crouching);
    assert_eq!(state.desired_stance, ControllerStance::Crouching);
    assert!(state.forward_speed <= config.crouch_speed);
}

#[test]
fn plugin_retries_uncrouch_after_leaving_low_ceiling_without_jump() {
    let mut app = controller_app();
    let config = FirstPersonControllerConfig::default();
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            FirstPersonMotorState {
                stance: ControllerStance::Crouching,
                desired_stance: ControllerStance::Crouching,
                ..default()
            },
            Transform::from_xyz(0.0, config.height(ControllerStance::Crouching) * 0.5, 0.0),
        ))
        .id();
    app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(2.0, 0.2, 2.0),
        Transform::from_xyz(0.0, 1.45, 0.0),
    ));
    app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(8.0, 0.1, 8.0),
        Transform::from_xyz(0.0, -0.05, 0.0),
    ));

    app.update();
    test_input::clear_input(&mut app, player);
    app.update();
    assert_eq!(
        app.world()
            .get::<FirstPersonMotorState>(player)
            .unwrap()
            .stance,
        ControllerStance::Crouching
    );

    app.world_mut()
        .get_mut::<Transform>(player)
        .unwrap()
        .translation
        .x = 3.0;
    test_input::clear_input(&mut app, player);
    app.update();

    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    let transform = app.world().get::<Transform>(player).unwrap();
    assert_eq!(state.stance, ControllerStance::Standing);
    assert!(
        (transform.translation.y - config.height(ControllerStance::Standing) * 0.5).abs() < 0.002
    );
}

#[test]
fn plugin_uncrouch_tries_hpl2_side_offsets() {
    let mut app = controller_app();
    let config = FirstPersonControllerConfig::default();
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            FirstPersonMotorState {
                stance: ControllerStance::Crouching,
                desired_stance: ControllerStance::Crouching,
                ..default()
            },
            Transform::from_xyz(0.0, config.height(ControllerStance::Crouching) * 0.5, 0.0),
        ))
        .id();
    app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(0.004, 0.2, 0.05),
        Transform::from_xyz(-config.body_radius + 0.001, 1.45, 0.0),
    ));

    test_input::set_input(&mut app, player, command(&[], &[AfterglowAction::Crouch]));
    app.update();
    test_input::clear_input(&mut app, player);
    app.update();

    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    let transform = app.world().get::<Transform>(player).unwrap();
    assert_eq!(state.stance, ControllerStance::Standing);
    assert!(transform.translation.x > 0.005);
}

#[test]
fn plugin_keeps_feet_stable_when_entering_crouch() {
    let mut app = controller_app();
    let config = FirstPersonControllerConfig::default();
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(0.0, config.height(ControllerStance::Standing) * 0.5, 0.0),
        ))
        .id();
    app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(4.0, 0.1, 4.0),
        Transform::from_xyz(0.0, -0.05, 0.0),
    ));

    app.update();
    let before_transform = *app.world().get::<Transform>(player).unwrap();
    let before_state = *app.world().get::<FirstPersonMotorState>(player).unwrap();
    let before_feet_y = before_transform.translation.y - config.height(before_state.stance) * 0.5;
    test_input::set_input(&mut app, player, command(&[], &[AfterglowAction::Crouch]));
    app.update();

    let transform = app.world().get::<Transform>(player).unwrap();
    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    let feet_y = transform.translation.y - config.height(state.stance) * 0.5;
    assert_eq!(state.stance, ControllerStance::Crouching);
    assert!((feet_y - before_feet_y).abs() < 0.001);
}

fn controller_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AfterglowCorePlugin,
        AfterglowPhysicsPlugin,
        AfterglowFirstPersonControllerPlugin,
    ));
    app.finish();
    app.cleanup();
    *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
        TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 60.0));
    app
}
