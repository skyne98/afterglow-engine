use super::*;
use crate::{
    core::AfterglowCorePlugin,
    input::{InputActionValue, InputAxis, InputAxisValue, PlayerCommand, PlayerCommandQueue},
    physics::{AfterglowPhysicsPlugin, PhysicsBody, PhysicsCollider},
};
use bevy::time::TimeUpdateStrategy;
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
struct Sample {
    position: Vec3,
    climbing: bool,
    intent_forward_speed: f32,
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
    app.init_resource::<PlayerCommandQueue>();
    app.finish();
    app.cleanup();
    *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
        TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 60.0));
    app
}

fn move_right_command() -> PlayerCommand {
    PlayerCommand {
        player: NetworkPlayerId(1),
        axes: vec![InputAxisValue {
            axis: InputAxis::new("move.x"),
            value: 1.0,
        }],
        ..default()
    }
}

fn move_forward_command() -> PlayerCommand {
    PlayerCommand {
        player: NetworkPlayerId(1),
        axes: vec![InputAxisValue {
            axis: InputAxis::new("move.y"),
            value: 1.0,
        }],
        ..default()
    }
}

fn sprint_forward_command() -> PlayerCommand {
    PlayerCommand {
        player: NetworkPlayerId(1),
        axes: vec![InputAxisValue {
            axis: InputAxis::new("move.y"),
            value: 1.0,
        }],
        actions: vec![InputActionValue::held("sprint")],
        ..default()
    }
}

fn spawn_static_box(app: &mut App, size: Vec3, transform: Transform) {
    app.world_mut().spawn((
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(size),
        transform,
    ));
}

fn sample(app: &App, player: Entity) -> Sample {
    let position = app.world().get::<Transform>(player).unwrap().translation;
    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    let local_speed = body::local_speeds_from_velocity(state);
    Sample {
        position,
        climbing: state.climbing,
        intent_forward_speed: state.forward_speed,
        actual_forward_speed: local_speed.x,
        actual_side_speed: local_speed.y,
    }
}

fn range(samples: &[Sample], axis: fn(Vec3) -> f32) -> f32 {
    let min = samples
        .iter()
        .map(|sample| axis(sample.position))
        .fold(f32::INFINITY, f32::min);
    let max = samples
        .iter()
        .map(|sample| axis(sample.position))
        .fold(f32::NEG_INFINITY, f32::max);
    max - min
}

#[test]
fn controller_clips_failed_low_blocker_intent_after_sprint_reset() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let obstacle_height = config.max_step_height + 0.14;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                player: NetworkPlayerId(1),
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, 5.5),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 16.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(1.2, obstacle_height, 0.55),
        Transform::from_xyz(0.0, obstacle_height * 0.5, 0.2),
    );

    app.update();
    for _ in 0..24 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![sprint_forward_command()]);
        app.update();
    }
    for _ in 0..60 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(Vec::new());
        app.update();
    }
    assert!(
        app.world()
            .get::<FirstPersonMotorState>(player)
            .unwrap()
            .forward_speed
            .abs()
            < 0.01,
        "sprint warmup did not fully reset before blocker test"
    );

    let mut samples = Vec::with_capacity(160);
    for _ in 0..160 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![move_forward_command()]);
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[100..];
    assert!(
        settled.iter().all(|sample| !sample.climbing),
        "low blocker triggered stair climbing after sprint reset: {samples:?}"
    );
    assert!(
        range(settled, |position| position.x) < 0.003,
        "low blocker caused left/right jitter after sprint reset: {samples:?}"
    );
    assert!(
        settled
            .iter()
            .all(|sample| sample.actual_forward_speed.abs() < 0.05),
        "low blocker kept actual forward motion after sprint reset: {samples:?}"
    );
    assert!(
        settled
            .iter()
            .all(|sample| (sample.intent_forward_speed - 5.0).abs() < 0.1),
        "failed low-blocker intent is not sustained at ground_speed (HPL2 does not clip intent on failed steps): {samples:?}"
    );
}

#[test]
fn controller_does_not_jitter_against_low_centered_blocker() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let obstacle_height = config.max_step_height + 0.14;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                player: NetworkPlayerId(1),
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
        Vec3::new(config.body_radius * 2.0, obstacle_height, 0.55),
        Transform::from_xyz(0.0, obstacle_height * 0.5, 0.2),
    );

    app.update();
    let mut samples = Vec::with_capacity(121);
    for _ in 0..120 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![move_forward_command()]);
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[60..];
    assert!(
        settled.iter().all(|sample| !sample.climbing),
        "low centered blocker triggered stair climbing: {samples:?}"
    );
    assert!(
        range(settled, |position| position.x) < 0.003,
        "low centered blocker caused left/right jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.y) < 0.003,
        "low centered blocker caused vertical jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.z) < 0.006,
        "low centered blocker caused blocked-axis jitter: {samples:?}"
    );
}

#[test]
fn controller_does_not_jitter_against_thick_knee_wall() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let obstacle_height = 0.5;
    let wall_half_depth = 2.0;
    let wall_center_z = 0.0;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                player: NetworkPlayerId(1),
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, wall_center_z + wall_half_depth + 2.0),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 16.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, obstacle_height, wall_half_depth),
        Transform::from_xyz(0.0, obstacle_height * 0.5, wall_center_z),
    );

    app.update();
    let mut samples = Vec::with_capacity(200);
    for _ in 0..200 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![move_forward_command()]);
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[120..];
    assert!(
        settled.iter().all(|sample| !sample.climbing),
        "thick knee wall triggered stair climbing: {samples:?}"
    );
    assert!(
        range(settled, |position| position.x) < 0.003,
        "thick knee wall caused left/right jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.y) < 0.003,
        "thick knee wall caused vertical jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.z) < 0.01,
        "thick knee wall caused forward/back jitter: {samples:?}"
    );
}

#[test]
fn controller_does_not_jitter_against_chest_high_barrier() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let obstacle_height = 0.9;
    let wall_half_depth = 2.0;
    let wall_center_z = 0.0;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                player: NetworkPlayerId(1),
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, wall_center_z + wall_half_depth + 2.0),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 16.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, obstacle_height, wall_half_depth),
        Transform::from_xyz(0.0, obstacle_height * 0.5, wall_center_z),
    );

    app.update();
    let mut samples = Vec::with_capacity(200);
    for _ in 0..200 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![move_forward_command()]);
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[120..];
    assert!(
        settled.iter().all(|sample| !sample.climbing),
        "chest-high barrier triggered stair climbing: {samples:?}"
    );
    assert!(
        range(settled, |position| position.x) < 0.003,
        "chest-high barrier caused left/right jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.y) < 0.003,
        "chest-high barrier caused vertical jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.z) < 0.01,
        "chest-high barrier caused forward/back jitter: {samples:?}"
    );
}

#[test]
fn controller_does_not_jitter_against_waist_high_barrier() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let obstacle_height = 0.36; // exactly at max_step_height
    let wall_half_depth = 2.0;
    let wall_center_z = 0.0;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                player: NetworkPlayerId(1),
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, wall_center_z + wall_half_depth + 2.0),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 16.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, obstacle_height, wall_half_depth),
        Transform::from_xyz(0.0, obstacle_height * 0.5, wall_center_z),
    );

    app.update();
    let mut samples = Vec::with_capacity(200);
    for _ in 0..200 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![move_forward_command()]);
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[120..];
    assert!(
        settled.iter().all(|sample| !sample.climbing),
        "waist-high barrier at max_step_height triggered stair climbing: {samples:?}"
    );
    assert!(
        range(settled, |position| position.x) < 0.003,
        "waist-high barrier caused left/right jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.y) < 0.003,
        "waist-high barrier caused vertical jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.z) < 0.01,
        "waist-high barrier caused forward/back jitter: {samples:?}"
    );
}

#[test]
fn controller_does_not_jitter_against_demo_accent_box_centered() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let box_size = Vec3::new(1.5, 0.5, 3.0);
    let box_translation = Vec3::new(2.5, 0.25, -2.0);
    let box_front_z = box_translation.z + box_size.z * 0.5;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                player: NetworkPlayerId(1),
                config: config.clone(),
            },
            Transform::from_xyz(box_translation.x, half_height, box_front_z + 2.0),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(28.0, 0.4, 28.0),
        Transform::from_xyz(0.0, -0.2, 0.0),
    );
    spawn_static_box(
        &mut app,
        box_size,
        Transform::from_translation(box_translation),
    );

    app.update();
    let mut samples = Vec::with_capacity(200);
    for _ in 0..200 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![move_forward_command()]);
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[120..];
    assert!(
        settled.iter().all(|sample| !sample.climbing),
        "demo accent box centered triggered stair climbing: {samples:?}"
    );
    assert!(
        range(settled, |position| position.x) < 0.003,
        "demo accent box centered caused left/right jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.z) < 0.01,
        "demo accent box centered caused forward/back jitter: {samples:?}"
    );
}

#[test]
fn controller_does_not_jitter_against_demo_accent_box() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let box_size = Vec3::new(1.5, 0.5, 3.0);
    let box_translation = Vec3::new(2.5, 0.25, -2.0);
    let box_front_z = box_translation.z + box_size.z * 0.5;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                player: NetworkPlayerId(1),
                config: config.clone(),
            },
            Transform::from_xyz(box_translation.x - 0.05, half_height, box_front_z + 2.0),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(28.0, 0.4, 28.0),
        Transform::from_xyz(0.0, -0.2, 0.0),
    );
    spawn_static_box(
        &mut app,
        box_size,
        Transform::from_translation(box_translation),
    );

    app.update();
    let mut samples = Vec::with_capacity(200);
    for _ in 0..200 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![move_forward_command()]);
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[120..];
    assert!(
        settled.iter().all(|sample| !sample.climbing),
        "demo accent box triggered stair climbing: {samples:?}"
    );
    assert!(
        range(settled, |position| position.x) < 0.003,
        "demo accent box caused left/right jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.z) < 0.01,
        "demo accent box caused forward/back jitter: {samples:?}"
    );
}

#[test]
fn controller_does_not_jitter_against_demo_barrier() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let box_size = Vec3::new(2.2, 0.45, 0.55);
    let box_translation = Vec3::new(-4.0, 0.225, -0.5);
    let box_front_z = box_translation.z + box_size.z * 0.5;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                player: NetworkPlayerId(1),
                config: config.clone(),
            },
            Transform::from_xyz(box_translation.x, half_height, box_front_z + 2.0),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(28.0, 0.4, 28.0),
        Transform::from_xyz(0.0, -0.2, 0.0),
    );
    spawn_static_box(
        &mut app,
        box_size,
        Transform::from_translation(box_translation),
    );

    app.update();
    let mut samples = Vec::with_capacity(200);
    for _ in 0..200 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![move_forward_command()]);
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[120..];
    assert!(
        settled.iter().all(|sample| !sample.climbing),
        "demo barrier triggered stair climbing: {samples:?}"
    );
    assert!(
        range(settled, |position| position.x) < 0.003,
        "demo barrier caused left/right jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.z) < 0.01,
        "demo barrier caused forward/back jitter: {samples:?}"
    );
}

#[test]
fn controller_does_not_jitter_sideways_against_low_blocker() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let obstacle_height = config.max_step_height + 0.14;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                player: NetworkPlayerId(1),
                config: config.clone(),
            },
            Transform::from_xyz(-0.2, half_height, 0.0),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 10.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(0.55, obstacle_height, 3.0),
        Transform::from_xyz(0.55, obstacle_height * 0.5, 0.0),
    );

    app.update();
    let mut samples = Vec::with_capacity(121);
    for _ in 0..120 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![move_right_command()]);
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[60..];
    assert!(
        settled.iter().all(|sample| !sample.climbing),
        "low side blocker triggered stair climbing: {samples:?}"
    );
    assert!(
        range(settled, |position| position.x) < 0.006,
        "low side blocker caused blocked-axis jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.y) < 0.003,
        "low side blocker caused vertical jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.z) < 0.003,
        "low side blocker caused left/right jitter along the wall: {samples:?}"
    );
    assert!(
        settled
            .iter()
            .all(|sample| sample.actual_side_speed.abs() < 0.05),
        "low side blocker kept reporting side motion for camera/presentation: {samples:?}"
    );
}

#[test]
fn controller_does_not_jitter_against_tallest_stair_side() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let step_height = 0.72;
    let step_half_width = 1.1;
    let side_face_x = -step_half_width;
    let step_z = -0.25;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                player: NetworkPlayerId(1),
                config: config.clone(),
            },
            Transform::from_xyz(side_face_x - 2.0, half_height, step_z),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(28.0, 0.4, 28.0),
        Transform::from_xyz(0.0, -0.2, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(2.2, step_height, 0.55),
        Transform::from_xyz(0.0, step_height * 0.5, step_z),
    );

    app.update();
    let mut samples = Vec::with_capacity(200);
    for _ in 0..200 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![move_right_command()]);
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[120..];
    assert!(
        settled.iter().all(|sample| !sample.climbing),
        "tallest stair side face triggered stair climbing: {samples:?}"
    );
    assert!(
        range(settled, |position| position.x) < 0.006,
        "tallest stair side face caused blocked-axis jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.z) < 0.003,
        "tallest stair side face caused left/right jitter along the wall: {samples:?}"
    );
}

#[test]
fn centered_blocker_exact_stillness_check() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let obstacle_height = config.max_step_height + 0.14;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                player: NetworkPlayerId(1),
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
        Vec3::new(config.body_radius * 2.0, obstacle_height, 0.55),
        Transform::from_xyz(0.0, obstacle_height * 0.5, 0.2),
    );

    app.update();
    let mut samples = Vec::with_capacity(180);
    for _ in 0..180 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![move_forward_command()]);
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[120..];
    let first = settled[0];
    eprintln!(
        "centered: settled pos ({}, {}, {})",
        first.position.x, first.position.y, first.position.z
    );
    eprintln!(
        "x_range={} y_range={} z_range={}",
        range(settled, |p| p.x),
        range(settled, |p| p.y),
        range(settled, |p| p.z),
    );
    assert!(
        settled.iter().all(|s| s.position == first.position),
        "centered blocker not exactly still: converged on ({}, {}, {})",
        first.position.x,
        first.position.y,
        first.position.z
    );
}

#[test]
fn offcenter_accent_box_exact_stillness_check() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let box_size = Vec3::new(1.5, 0.5, 3.0);
    let box_translation = Vec3::new(2.5, 0.25, -2.0);
    let box_front_z = box_translation.z + box_size.z * 0.5;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                player: NetworkPlayerId(1),
                config: config.clone(),
            },
            Transform::from_xyz(box_translation.x - 0.05, half_height, box_front_z + 2.0),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(28.0, 0.4, 28.0),
        Transform::from_xyz(0.0, -0.2, 0.0),
    );
    spawn_static_box(
        &mut app,
        box_size,
        Transform::from_translation(box_translation),
    );

    app.update();
    let mut samples = Vec::with_capacity(300);
    for _ in 0..300 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![move_forward_command()]);
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[200..];
    let first = settled[0];
    eprintln!(
        "off-center: settled pos ({}, {}, {})",
        first.position.x, first.position.y, first.position.z
    );
    eprintln!(
        "x_range={} y_range={} z_range={}",
        range(settled, |p| p.x),
        range(settled, |p| p.y),
        range(settled, |p| p.z),
    );
    for (i, s) in settled.iter().enumerate() {
        if s.position.x != first.position.x {
            eprintln!(
                "  frame {} x diverged: {} vs {}",
                i + 200,
                s.position.x,
                first.position.x
            );
            break;
        }
    }
    assert!(
        settled.iter().all(|s| s.position == first.position),
        "off-center box not exactly still: converged on ({}, {}, {})",
        first.position.x,
        first.position.y,
        first.position.z
    );
}
