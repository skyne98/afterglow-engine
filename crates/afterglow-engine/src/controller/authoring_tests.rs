use super::*;
use crate::{
    core::AfterglowCorePlugin,
    input::{InputActionValue, InputAxis, InputAxisValue, PlayerCommand, PlayerCommandQueue},
    physics::{AfterglowPhysicsPlugin, PhysicsBody, PhysicsCollider},
};
use avian3d::prelude::{Collider, RigidBody};

fn command(axes: &[(&str, f32)], actions: &[InputActionValue]) -> PlayerCommand {
    PlayerCommand {
        player: NetworkPlayerId(1),
        axes: axes
            .iter()
            .map(|(axis, value)| InputAxisValue {
                axis: InputAxis::new(*axis),
                value: *value,
            })
            .collect(),
        actions: actions.to_vec(),
        ..default()
    }
}

#[test]
fn plugin_authors_kinematic_cylinder_for_controller() {
    let mut app = controller_app();
    app.init_resource::<PlayerCommandQueue>();
    let entity = app
        .world_mut()
        .spawn((
            FirstPersonController::new(NetworkPlayerId(1)),
            Transform::from_xyz(0.0, 1.0, 0.0),
        ))
        .id();
    let crouched = app
        .world_mut()
        .spawn((
            FirstPersonController::new(NetworkPlayerId(2)),
            FirstPersonMotorState {
                stance: ControllerStance::Crouching,
                desired_stance: ControllerStance::Crouching,
                ..default()
            },
            Transform::from_xyz(0.0, 1.0, 0.0),
        ))
        .id();
    app.world_mut()
        .resource_mut::<PlayerCommandQueue>()
        .replace(vec![PlayerCommand {
            player: NetworkPlayerId(2),
            actions: vec![InputActionValue::held("crouch")],
            ..default()
        }]);

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
    app.init_resource::<PlayerCommandQueue>();
    let config = FirstPersonControllerConfig::default();
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                player: NetworkPlayerId(1),
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

    app.world_mut()
        .resource_mut::<PlayerCommandQueue>()
        .replace(vec![command(
            &[],
            &[InputActionValue::held(config.crouch_action.clone())],
        )]);
    app.update();
    app.world_mut()
        .resource_mut::<PlayerCommandQueue>()
        .replace(vec![command(&[("move.y", 1.0)], &[])]);
    app.update();

    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    assert_eq!(state.stance, ControllerStance::Crouching);
    assert_eq!(state.desired_stance, ControllerStance::Crouching);
    assert!(state.forward_speed <= config.crouch_speed);
}

#[test]
fn plugin_retries_uncrouch_after_leaving_low_ceiling_without_jump() {
    let mut app = controller_app();
    app.init_resource::<PlayerCommandQueue>();
    let config = FirstPersonControllerConfig::default();
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                player: NetworkPlayerId(1),
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

    app.update();
    app.world_mut()
        .resource_mut::<PlayerCommandQueue>()
        .replace(vec![command(&[], &[])]);
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
    app.world_mut()
        .resource_mut::<PlayerCommandQueue>()
        .replace(vec![command(&[], &[])]);
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
    app.init_resource::<PlayerCommandQueue>();
    let config = FirstPersonControllerConfig::default();
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                player: NetworkPlayerId(1),
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

    app.world_mut()
        .resource_mut::<PlayerCommandQueue>()
        .replace(vec![command(
            &[],
            &[InputActionValue::held(config.crouch_action.clone())],
        )]);
    app.update();
    app.world_mut()
        .resource_mut::<PlayerCommandQueue>()
        .replace(vec![command(&[], &[])]);
    app.update();

    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    let transform = app.world().get::<Transform>(player).unwrap();
    assert_eq!(state.stance, ControllerStance::Standing);
    assert!(transform.translation.x > 0.005);
}

#[test]
fn plugin_keeps_feet_stable_when_entering_crouch() {
    let mut app = controller_app();
    app.init_resource::<PlayerCommandQueue>();
    let config = FirstPersonControllerConfig::default();
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                player: NetworkPlayerId(1),
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
    app.world_mut()
        .resource_mut::<PlayerCommandQueue>()
        .replace(vec![command(
            &[],
            &[InputActionValue::pressed(config.crouch_action.clone())],
        )]);
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
    app
}
