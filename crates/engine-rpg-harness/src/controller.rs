use afterglow_engine::{
    controller::{
        AfterglowFirstPersonControllerPlugin, ControllerStance, FirstPersonController,
        FirstPersonControllerConfig, FirstPersonMotorState,
    },
    core::AfterglowCorePlugin,
    input::AfterglowAction,
    physics::{
        AfterglowPhysicsPlugin, PhysicsBody, PhysicsCollider,
        avian::{Collider, RigidBody},
    },
};
use bevy::{prelude::*, time::TimeUpdateStrategy};
use leafwing_input_manager::action_state::ActionState;
use std::time::Duration;

fn set_input(app: &mut App, player: Entity, state: ActionState<AfterglowAction>) {
    app.world_mut().entity_mut(player).insert(state);
}

fn command(axes: &[(&str, f32)], pressed: &[AfterglowAction]) -> ActionState<AfterglowAction> {
    let mut move_axis = Vec2::ZERO;
    let mut look_axis = Vec2::ZERO;
    for (axis, value) in axes {
        match *axis {
            "move.x" => move_axis.x = *value,
            "move.y" => move_axis.y = *value,
            "look.x" => look_axis.x = *value,
            "look.y" => look_axis.y = *value,
            _ => unreachable!("unknown test axis: {axis}"),
        }
    }
    let mut state = ActionState::default();
    state.set_axis_pair(&AfterglowAction::Move, move_axis);
    state.set_axis_pair(&AfterglowAction::Look, look_axis);
    for action in pressed {
        state.press(action);
    }
    state
}

fn app() -> App {
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

fn spawn_static_box(app: &mut App, size: Vec3, transform: Transform) {
    app.world_mut().spawn((
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(size),
        transform,
    ));
}

fn settle(app: &mut App, _player: Entity) {
    for _ in 0..4 {
        app.update();
    }
}

#[test]
fn player_moves_forward_with_controller_and_physics() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, 0.0),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 8.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );

    settle(&mut app, player);

    assert!(
        app.world().get::<RigidBody>(player).is_some(),
        "player should have RigidBody after authoring"
    );
    assert!(
        app.world().get::<Collider>(player).is_some(),
        "player should have Collider after authoring"
    );

    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    assert!(state.grounded, "player should be grounded after spawn");

    for _ in 0..60 {
        set_input(&mut app, player, command(&[("move.y", 1.0)], &[]));
        app.update();
    }

    let pos = app.world().get::<Transform>(player).unwrap().translation;
    assert!(
        pos.z < -2.0,
        "player should move forward at least 2 units, was at z={}",
        pos.z
    );
}

#[test]
fn player_jumps_and_lands() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, 0.0),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 8.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );

    settle(&mut app, player);

    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    assert!(state.grounded, "player should be grounded before jump");

    for _ in 0..3 {
        set_input(
            &mut app,
            player,
            command(&[("move.y", 1.0)], &[AfterglowAction::Jump]),
        );
        app.update();
    }

    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    assert!(!state.grounded, "player should have left ground after jump");

    for _ in 0..180 {
        set_input(&mut app, player, command(&[], &[]));
        app.update();
        if app
            .world()
            .get::<FirstPersonMotorState>(player)
            .unwrap()
            .grounded
        {
            break;
        }
    }

    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    assert!(state.grounded, "player should be grounded after landing");
}

#[test]
fn player_look_rotates_yaw() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, 0.0),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 8.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    app.update();

    for _ in 0..10 {
        set_input(&mut app, player, command(&[("look.x", 1.0)], &[]));
        app.update();
    }

    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    assert!(
        state.yaw < 0.0,
        "yaw should be negative after looking right, was {}",
        state.yaw
    );

    let yaw_after_look = state.yaw;

    for _ in 0..5 {
        set_input(&mut app, player, command(&[], &[]));
        app.update();
    }

    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    assert!(
        (state.yaw - yaw_after_look).abs() < 0.0001,
        "yaw should stay constant without look input, was {}",
        state.yaw
    );
}

#[test]
fn player_crouches_under_ceiling() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, 0.0),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 8.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    // Ceiling: standing top (~1.8m) does not fit, crouching top (~1.15m) fits.
    // Bottom at y=1.3; z-range [-18, -2] so spawn at z=0 does NOT intersect.
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 16.0),
        Transform::from_xyz(0.0, 1.4, -10.0),
    );

    settle(&mut app, player);

    // Apply crouch and move forward under the ceiling
    set_input(
        &mut app,
        player,
        command(&[("move.y", 1.0)], &[AfterglowAction::Crouch]),
    );
    app.update();

    let s = app.world().get::<FirstPersonMotorState>(player).unwrap();
    assert!(s.grounded);
    assert_eq!(s.stance, ControllerStance::Crouching);

    for _ in 0..100 {
        set_input(
            &mut app,
            player,
            command(&[("move.y", 1.0)], &[AfterglowAction::Crouch]),
        );
        app.update();
    }

    let pos = app.world().get::<Transform>(player).unwrap().translation;
    let s = app.world().get::<FirstPersonMotorState>(player).unwrap();
    assert!(
        s.grounded,
        "crouched player should stay grounded, y={}",
        pos.y
    );
    assert!(
        pos.z < -2.0,
        "crouched player should move forward under ceiling, was z={}",
        pos.z
    );

    // Release crouch — still under ceiling, cannot stand back up
    for _ in 0..8 {
        set_input(&mut app, player, command(&[("move.y", 1.0)], &[]));
        app.update();
    }

    let s = app.world().get::<FirstPersonMotorState>(player).unwrap();
    assert_eq!(
        s.stance,
        ControllerStance::Crouching,
        "player should stay crouching under low ceiling"
    );
}

#[test]
fn wall_stops_movement() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, 0.0),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 8.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    // Wall 1.5 units ahead. Front face at z=-1.4, player body radius 0.35 → stop at
    // z≈-1.05.
    spawn_static_box(
        &mut app,
        Vec3::new(4.0, 2.0, 0.2),
        Transform::from_xyz(0.0, 1.0, -1.5),
    );

    settle(&mut app, player);

    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    assert!(
        state.grounded,
        "player should be grounded before hitting wall"
    );

    let start_pos = app.world().get::<Transform>(player).unwrap().translation;

    for _ in 0..120 {
        set_input(&mut app, player, command(&[("move.y", 1.0)], &[]));
        app.update();
    }

    let pos = app.world().get::<Transform>(player).unwrap().translation;
    assert!(
        pos.z < start_pos.z - 0.3,
        "player should move toward wall, was z={}",
        pos.z
    );
    assert!(
        pos.z > -1.3,
        "player should not clip through wall, was at z={}",
        pos.z
    );

    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    assert!(
        state.grounded,
        "player should still be grounded against wall"
    );
}
