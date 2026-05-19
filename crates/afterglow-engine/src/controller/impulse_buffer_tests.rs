use super::*;
use crate::{
    core::AfterglowCorePlugin,
    physics::{AfterglowPhysicsPlugin, PhysicsBody, PhysicsCollider},
};
use bevy::time::TimeUpdateStrategy;
use std::time::Duration;

fn app_with_dt(seconds: f64) -> App {
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
        TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(seconds));
    app
}

fn spawn_floor(app: &mut App) {
    app.world_mut().spawn((
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(Vec3::new(200.0, 0.2, 200.0)),
        Transform::from_xyz(0.0, -0.1, 0.0),
    ));
}

#[test]
fn plugin_authors_default_impulse_buffer_for_controller() {
    let mut app = app_with_dt(1.0 / 60.0);
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController::new(),
            Transform::from_xyz(0.0, 0.9, 0.0),
        ))
        .id();

    app.update();

    assert!(
        app.world()
            .get::<FirstPersonImpulseBuffer>(player)
            .is_some(),
        "controller authoring should add an impulse buffer for combat forces"
    );
}

#[test]
fn impulse_accumulates_multiple_sources_clamps_and_clears() {
    let mut buffer = FirstPersonImpulseBuffer {
        max_linear_velocity_delta: 3.0,
        ..default()
    };

    buffer.add_linear_impulse(Vec3::X * 2.0);
    buffer.add_linear_impulse(Vec3::Y * 4.0);
    buffer.add_linear_impulse(Vec3::new(f32::NAN, 0.0, 0.0));

    let drained = buffer.drain_linear_impulse();
    assert!((drained.length() - 3.0).abs() < 0.001);
    assert!(drained.x > 0.0 && drained.y > 0.0);
    assert_eq!(buffer.linear_velocity_delta, Vec3::ZERO);
    assert_eq!(buffer.drain_linear_impulse(), Vec3::ZERO);
}

#[test]
fn impulse_drain_rejects_invalid_accumulator_and_non_positive_caps() {
    let mut zero_cap = FirstPersonImpulseBuffer {
        linear_velocity_delta: Vec3::X,
        max_linear_velocity_delta: 0.0,
    };
    let mut negative_cap = FirstPersonImpulseBuffer {
        linear_velocity_delta: Vec3::X,
        max_linear_velocity_delta: -4.0,
    };
    let mut invalid_accumulator = FirstPersonImpulseBuffer {
        linear_velocity_delta: Vec3::new(f32::INFINITY, 0.0, 0.0),
        ..default()
    };

    assert_eq!(zero_cap.drain_linear_impulse(), Vec3::ZERO);
    assert_eq!(negative_cap.drain_linear_impulse(), Vec3::ZERO);
    assert_eq!(invalid_accumulator.drain_linear_impulse(), Vec3::ZERO);
    assert_eq!(invalid_accumulator.linear_velocity_delta, Vec3::ZERO);
}

#[test]
fn horizontal_impulse_moves_controller_in_world_space_next_fixed_tick() {
    let mut app = app_with_dt(1.0 / 60.0);
    let config = FirstPersonControllerConfig::default();
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            FirstPersonMotorState {
                yaw: 1.1,
                ..default()
            },
            Transform::from_xyz(0.0, config.height(ControllerStance::Standing) * 0.5, 0.0),
        ))
        .id();
    spawn_floor(&mut app);
    app.update();

    app.world_mut()
        .get_mut::<FirstPersonImpulseBuffer>(player)
        .unwrap()
        .add_linear_impulse(Vec3::X * 6.0);
    let before = app.world().get::<Transform>(player).unwrap().translation;
    app.update();
    let after = app.world().get::<Transform>(player).unwrap().translation;

    assert!(
        after.x > before.x + 0.05,
        "world-space impulse should move the body along its world direction: {before:?} -> {after:?}"
    );
    assert!(
        app.world()
            .get::<FirstPersonImpulseBuffer>(player)
            .unwrap()
            .linear_velocity_delta
            .length()
            < 0.001
    );
}

#[test]
fn horizontal_impulse_direction_is_world_space_not_view_space() {
    let mut app = app_with_dt(1.0 / 60.0);
    let config = FirstPersonControllerConfig::default();
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            FirstPersonMotorState {
                yaw: std::f32::consts::FRAC_PI_2,
                ..default()
            },
            Transform::from_xyz(0.0, config.height(ControllerStance::Standing) * 0.5, 0.0),
        ))
        .id();
    spawn_floor(&mut app);
    app.update();

    app.world_mut()
        .get_mut::<FirstPersonImpulseBuffer>(player)
        .unwrap()
        .add_linear_impulse(Vec3::Z * 6.0);
    let before = app.world().get::<Transform>(player).unwrap().translation;
    app.update();
    let after = app.world().get::<Transform>(player).unwrap().translation;

    assert!(
        after.z > before.z + 0.05,
        "impulse should push along world +Z even when the controller is facing sideways: {before:?} -> {after:?}"
    );
    assert!(
        (after.x - before.x).abs() < 0.01,
        "world +Z impulse should not leak into world X at yaw 90 degrees: {before:?} -> {after:?}"
    );
}

#[test]
fn upward_impulse_combines_with_gravity_and_does_not_repeat() {
    let mut app = app_with_dt(1.0 / 60.0);
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
    spawn_floor(&mut app);
    app.update();

    app.world_mut()
        .get_mut::<FirstPersonImpulseBuffer>(player)
        .unwrap()
        .add_linear_impulse(Vec3::Y * 3.0);
    app.update();
    let first_tick_velocity = app
        .world()
        .get::<FirstPersonMotorState>(player)
        .unwrap()
        .velocity
        .y;
    app.update();
    let second_tick_velocity = app
        .world()
        .get::<FirstPersonMotorState>(player)
        .unwrap()
        .velocity
        .y;

    assert!(first_tick_velocity > 2.0);
    assert!(
        second_tick_velocity < first_tick_velocity,
        "the impulse should clear after one fixed tick and gravity should resume"
    );
}
