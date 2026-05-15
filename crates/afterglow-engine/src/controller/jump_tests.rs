use super::*;
use crate::{
    core::AfterglowCorePlugin,
    input::{InputActionValue, InputAxis, InputAxisValue, PlayerCommand, PlayerCommandQueue},
    physics::{AfterglowPhysicsPlugin, PhysicsBody, PhysicsCollider},
};
use bevy::time::TimeUpdateStrategy;
use std::time::Duration;

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
fn disabled_jump_ignores_pressed_jump_action() {
    let config = FirstPersonControllerConfig {
        jump_enabled: false,
        ..default()
    };
    let mut state = FirstPersonMotorState {
        grounded: true,
        ..default()
    };
    let command = command(&[], &[InputActionValue::pressed("jump")]);

    integrate_first_person_motor(Some(&command), &config, &mut state, 1.0 / 60.0);

    assert!(state.grounded);
    assert_eq!(state.jump_buffer_ticks, 0);
    assert!((state.velocity.y + config.gravity / 60.0).abs() < 0.001);
}

#[test]
fn first_observed_held_jump_still_buffers_once() {
    let config = FirstPersonControllerConfig::default();
    let mut state = FirstPersonMotorState {
        grounded: true,
        ..default()
    };
    let command = command(&[], &[InputActionValue::held(config.jump_action.clone())]);

    integrate_first_person_motor(Some(&command), &config, &mut state, 1.0 / 60.0);

    assert!(!state.grounded);
    assert_eq!(state.velocity.y, config.jump_speed);
    assert!(state.jump_input_down);
}

#[test]
fn held_jump_does_not_repeat_without_release() {
    let config = FirstPersonControllerConfig::default();
    let mut state = FirstPersonMotorState {
        grounded: true,
        ..default()
    };
    let command = command(&[], &[InputActionValue::held(config.jump_action.clone())]);

    integrate_first_person_motor(Some(&command), &config, &mut state, 1.0 / 60.0);
    state.grounded = true;
    state.velocity.y = 0.0;
    integrate_first_person_motor(Some(&command), &config, &mut state, 1.0 / 60.0);

    assert!((state.velocity.y + config.gravity / 60.0).abs() < 0.001);
}

#[test]
fn disabled_jump_ignores_held_jump_gravity_relief() {
    let config = FirstPersonControllerConfig {
        jump_enabled: false,
        ..default()
    };
    let mut held = FirstPersonMotorState {
        velocity: Vec3::Y * config.jump_speed,
        jump_hold_ticks: config.jump_hold_ticks,
        ..default()
    };
    let mut released = held;
    let held_command = command(&[], &[InputActionValue::held(config.jump_action.clone())]);

    integrate_first_person_motor(Some(&held_command), &config, &mut held, 1.0 / 60.0);
    integrate_first_person_motor(None, &config, &mut released, 1.0 / 60.0);

    assert_eq!(held.velocity.y, released.velocity.y);
    assert_eq!(held.jump_hold_ticks, 0);
}

#[test]
fn steep_ground_cannot_start_or_refresh_jump() {
    let config = FirstPersonControllerConfig::default();
    let steep_angle = config.max_slope_angle + 0.1;
    let mut state = FirstPersonMotorState {
        grounded: true,
        ground_normal: Vec3::new(steep_angle.sin(), steep_angle.cos(), 0.0),
        coyote_ticks: config.coyote_ticks,
        ..default()
    };
    let command = command(
        &[],
        &[InputActionValue::pressed(config.jump_action.clone())],
    );

    integrate_first_person_motor(Some(&command), &config, &mut state, 1.0 / 60.0);

    assert!(state.grounded);
    assert_eq!(state.coyote_ticks, 0);
    assert_eq!(state.jump_hold_ticks, 0);
    assert_ne!(state.velocity.y, config.jump_speed);
}

#[test]
fn high_fps_jump_leaves_ground_probe_range_in_real_controller() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AfterglowCorePlugin,
        AfterglowPhysicsPlugin,
        AfterglowFirstPersonControllerPlugin,
    ));
    app.init_resource::<PlayerCommandQueue>();
    app.finish();
    app.cleanup();
    *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
        TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 60.0));

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
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(Vec3::new(16.0, 0.2, 16.0)),
        Transform::from_xyz(0.0, -0.1, 0.0),
    ));

    app.update();
    *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
        TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 240.0));
    app.world_mut()
        .resource_mut::<PlayerCommandQueue>()
        .replace(vec![command(
            &[],
            &[InputActionValue::pressed(config.jump_action.clone())],
        )]);

    for _ in 0..8 {
        app.update();
    }

    let transform = app.world().get::<Transform>(player).unwrap();
    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    assert!(
        transform.translation.y
            > config.height(ControllerStance::Standing) * 0.5 + config.ground_probe_distance,
        "high-FPS jump was snapped back to the ground: y={}, velocity={}",
        transform.translation.y,
        state.velocity.y
    );
    assert!(!state.grounded);
}

#[test]
fn sprint_jump_travels_farther_than_walk_jump() {
    let walk_distance = jump_distance(false);
    let sprint_distance = jump_distance(true);

    assert!(
        sprint_distance > walk_distance * 1.25,
        "sprint jump should travel farther: walk={walk_distance}, sprint={sprint_distance}"
    );
}

fn jump_distance(sprint: bool) -> f32 {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AfterglowCorePlugin,
        AfterglowPhysicsPlugin,
        AfterglowFirstPersonControllerPlugin,
    ));
    app.init_resource::<PlayerCommandQueue>();
    app.finish();
    app.cleanup();
    *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
        TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 60.0));

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
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(Vec3::new(64.0, 0.2, 64.0)),
        Transform::from_xyz(0.0, -0.1, 0.0),
    ));
    app.update();

    for _ in 0..45 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![move_command(&config, sprint, false)]);
        app.update();
    }
    app.world_mut()
        .resource_mut::<PlayerCommandQueue>()
        .replace(vec![move_command(&config, sprint, true)]);
    app.update();
    let jump_start = app.world().get::<Transform>(player).unwrap().translation;

    for _ in 0..20 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![move_command(&config, sprint, false)]);
        app.update();
    }

    let end = app.world().get::<Transform>(player).unwrap().translation;
    jump_start.z - end.z
}

fn move_command(config: &FirstPersonControllerConfig, sprint: bool, jump: bool) -> PlayerCommand {
    let mut actions = Vec::new();
    if sprint {
        actions.push(InputActionValue::held(config.sprint_action.clone()));
    }
    if jump {
        actions.push(InputActionValue::pressed(config.jump_action.clone()));
    }
    command(&[("move.y", 1.0)], &actions)
}
