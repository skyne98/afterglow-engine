use super::*;
use crate::{
    core::AfterglowCorePlugin,
    physics::{AfterglowPhysicsPlugin, PhysicsBody, PhysicsCollider},
};
use bevy::time::TimeUpdateStrategy;
use leafwing_input_manager::action_state::ActionState;
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
struct ControllerSample {
    position: Vec3,
    grounded: bool,
    climbing: bool,
    actual_forward_speed: f32,
    actual_side_speed: f32,
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

fn move_forward_command() -> ActionState<crate::input::AfterglowAction> {
    test_input::command(&[("move.y", 1.0)], &[])
}

fn spawn_static_box(app: &mut App, size: Vec3, transform: Transform) {
    app.world_mut().spawn((
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(size),
        transform,
    ));
}

fn set_frame_dt(app: &mut App, seconds: f64) {
    *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
        TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(seconds));
}

#[test]
fn grounded_controller_survives_large_frame_time_spike() {
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
    set_frame_dt(&mut app, 1.0);
    app.update();

    let sample = sample(&app, player);
    assert!(
        sample.grounded,
        "player lost ground after spike: {sample:?}"
    );
    assert!(
        sample.position.y >= half_height - 0.01,
        "large frame spike pushed player through floor: {sample:?}"
    );
}

fn sample(app: &App, player: Entity) -> ControllerSample {
    let position = app.world().get::<Transform>(player).unwrap().translation;
    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    let local_speed = body::local_speeds_from_velocity(state);
    ControllerSample {
        position,
        grounded: state.grounded,
        climbing: state.climbing,
        actual_forward_speed: local_speed.x,
        actual_side_speed: local_speed.y,
    }
}

#[test]
fn controller_smoothly_climbs_single_low_ledge() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, 1.4),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 10.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(2.0, 0.12, 10.0),
        Transform::from_xyz(0.0, 0.06, -4.5),
    );

    app.update();
    let mut samples = Vec::with_capacity(91);
    samples.push(sample(&app, player));
    for _ in 0..90 {
        test_input::set_input(&mut app, player, move_forward_command());
        app.update();
        samples.push(sample(&app, player));
    }

    let mut z_backtracks = 0;
    let mut large_down_snaps = 0;
    for pair in samples.windows(2) {
        let previous = pair[0];
        let current = pair[1];
        let dz = current.position.z - previous.position.z;
        let dy = current.position.y - previous.position.y;
        if dz > 0.002 {
            z_backtracks += 1;
        }
        if dy < -0.03 {
            large_down_snaps += 1;
        }
    }

    let start = samples.first().unwrap();
    let end = samples.last().unwrap();
    assert_eq!(
        z_backtracks, 0,
        "ledge approach moved backward: {samples:?}"
    );
    assert_eq!(
        large_down_snaps, 0,
        "ledge climb snapped downward: {samples:?}"
    );
    assert!(samples.iter().any(|sample| sample.climbing));
    assert!(samples.iter().filter(|sample| sample.grounded).count() > 80);
    assert!(end.position.z < start.position.z - 1.2);
    assert!(
        (end.position.y - (half_height + 0.12)).abs() < 0.02,
        "ledge top height was not reached smoothly: {samples:?}"
    );
}

#[test]
fn stair_step_up_sweep_reaches_landing_without_horizontal_stall() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, 1.4),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 10.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(2.0, 0.2, 10.0),
        Transform::from_xyz(0.0, 0.1, -4.5),
    );

    app.update();
    let mut max_climbing_dy: f32 = 0.0;
    let mut min_climbing_progress = f32::INFINITY;
    let mut previous = sample(&app, player);
    for _ in 0..120 {
        test_input::set_input(&mut app, player, move_forward_command());
        app.update();
        let current = sample(&app, player);
        if current.climbing || previous.climbing {
            let dy = current.position.y - previous.position.y;
            max_climbing_dy = max_climbing_dy.max(dy);
            min_climbing_progress =
                min_climbing_progress.min(previous.position.z - current.position.z);
        }
        previous = current;
    }
    let min_expected_progress = config.ground_speed / 60.0 * 0.35;

    assert!(
        max_climbing_dy <= config.max_step_height + config.step_climb_height_add + 0.001,
        "step-up exceeded configured step height: max_dy={max_climbing_dy}"
    );
    assert!(max_climbing_dy > 0.0, "test never observed a stair step-up");
    assert!(
        min_climbing_progress >= min_expected_progress,
        "step-up sweep stalled horizontally: min_progress={min_climbing_progress}, expected={min_expected_progress}"
    );
}

#[test]
fn controller_does_not_climb_ledge_above_step_height() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, 1.2),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 10.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(2.0, config.max_step_height + 0.24, 2.0),
        Transform::from_xyz(0.0, (config.max_step_height + 0.24) * 0.5, -1.0),
    );

    app.update();
    let mut samples = Vec::with_capacity(91);
    samples.push(sample(&app, player));
    for _ in 0..90 {
        test_input::set_input(&mut app, player, move_forward_command());
        app.update();
        samples.push(sample(&app, player));
    }

    let final_sample = *samples.last().unwrap();
    let min_y = samples
        .iter()
        .map(|sample| sample.position.y)
        .fold(f32::INFINITY, f32::min);
    let max_y = samples
        .iter()
        .map(|sample| sample.position.y)
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        final_sample.position.y < half_height + config.max_step_height * 0.5,
        "too-high ledge was climbed: {final_sample:?}"
    );
    assert!(
        final_sample.position.z > 0.3,
        "too-high ledge did not block forward movement: {final_sample:?}"
    );
    assert!(
        samples.iter().all(|sample| !sample.climbing),
        "too-high ledge triggered stair climbing: {samples:?}"
    );
    assert!(
        max_y - min_y < 0.003,
        "too-high ledge caused vertical jitter: {samples:?}"
    );
}

#[test]
fn controller_does_not_jitter_against_knee_height_blocker() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let obstacle_height = config.max_step_height + 0.06;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, 1.2),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 10.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(1.2, obstacle_height, 0.55),
        Transform::from_xyz(0.0, obstacle_height * 0.5, 0.2),
    );

    app.update();
    let mut samples = Vec::with_capacity(121);
    for _ in 0..120 {
        test_input::set_input(&mut app, player, move_forward_command());
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[60..];
    let min_y = settled
        .iter()
        .map(|sample| sample.position.y)
        .fold(f32::INFINITY, f32::min);
    let max_y = settled
        .iter()
        .map(|sample| sample.position.y)
        .fold(f32::NEG_INFINITY, f32::max);
    let min_z = settled
        .iter()
        .map(|sample| sample.position.z)
        .fold(f32::INFINITY, f32::min);
    let max_z = settled
        .iter()
        .map(|sample| sample.position.z)
        .fold(f32::NEG_INFINITY, f32::max);
    let min_x = settled
        .iter()
        .map(|sample| sample.position.x)
        .fold(f32::INFINITY, f32::min);
    let max_x = settled
        .iter()
        .map(|sample| sample.position.x)
        .fold(f32::NEG_INFINITY, f32::max);

    assert!(
        settled.iter().all(|sample| !sample.climbing),
        "knee-height blocker triggered stair climbing: {samples:?}"
    );
    assert!(
        max_y - min_y < 0.003,
        "knee-height blocker caused vertical jitter: {samples:?}"
    );
    assert!(
        max_z - min_z < 0.006,
        "knee-height blocker caused forward/back jitter: {samples:?}"
    );
    assert!(
        max_x - min_x < 0.003,
        "knee-height blocker caused left/right jitter: {samples:?}"
    );
    assert!(
        settled
            .iter()
            .all(|sample| sample.actual_forward_speed.abs() < 0.05),
        "blocked controller kept reporting forward motion for camera/presentation: {samples:?}"
    );
    assert!(
        settled
            .iter()
            .all(|sample| sample.actual_side_speed.abs() < 0.05),
        "blocked controller kept reporting side motion for camera/presentation: {samples:?}"
    );
}

#[test]
fn controller_smoothly_climbs_near_limit_knee_height_step() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let step_height = config.max_step_height - 0.02;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, 1.6),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 10.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(2.0, step_height, 14.0),
        Transform::from_xyz(0.0, step_height * 0.5, -6.5),
    );

    app.update();
    let mut samples = Vec::with_capacity(121);
    for _ in 0..120 {
        test_input::set_input(&mut app, player, move_forward_command());
        app.update();
        samples.push(sample(&app, player));
    }

    let mut z_backtracks = 0;
    let mut large_down_snaps = 0;
    for pair in samples.windows(2) {
        let previous = pair[0];
        let current = pair[1];
        let dz = current.position.z - previous.position.z;
        let dy = current.position.y - previous.position.y;
        if dz > 0.002 {
            z_backtracks += 1;
        }
        if dy < -0.03 {
            large_down_snaps += 1;
        }
    }
    let end = samples.last().unwrap();

    assert_eq!(z_backtracks, 0, "step climb backtracked: {samples:?}");
    assert_eq!(
        large_down_snaps, 0,
        "step climb snapped downward: {samples:?}"
    );
    assert!(
        samples.iter().any(|sample| sample.climbing),
        "near-limit step was never accepted: {samples:?}"
    );
    assert!(
        (end.position.y - (half_height + step_height)).abs() < 0.025,
        "near-limit step ended at wrong height: {samples:?}"
    );
}

#[test]
fn controller_step_up_sweep_respects_low_ceiling() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, 1.2),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 10.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(2.0, 0.12, 2.0),
        Transform::from_xyz(0.0, 0.06, -1.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(2.4, 0.2, 3.0),
        Transform::from_xyz(0.0, half_height * 2.0 + 0.16, -0.6),
    );

    app.update();
    for _ in 0..90 {
        test_input::set_input(&mut app, player, move_forward_command());
        app.update();
    }

    let final_sample = sample(&app, player);
    assert!(
        final_sample.position.y < half_height + 0.06,
        "low ceiling allowed stair up-sweep: {final_sample:?}"
    );
    assert!(
        final_sample.position.z > 0.2,
        "low-ceiling step did not block forward movement: {final_sample:?}"
    );
}
