use super::*;
use crate::{
    core::AfterglowCorePlugin,
    input::{InputAxis, InputAxisValue, PlayerCommand, PlayerCommandQueue},
    physics::{AfterglowPhysicsPlugin, PhysicsBody, PhysicsCollider},
};
use bevy::time::TimeUpdateStrategy;
use std::time::Duration;

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

fn spawn_player(app: &mut App, config: &FirstPersonControllerConfig, pos: Vec3) -> Entity {
    app.world_mut()
        .spawn((
            FirstPersonController {
                player: NetworkPlayerId(1),
                config: config.clone(),
            },
            Transform::from_translation(pos),
        ))
        .id()
}

fn spawn_static_box(app: &mut App, size: Vec3, transform: Transform) {
    app.world_mut().spawn((
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(size),
        transform,
    ));
}

fn run_forward(app: &mut App, frames: usize) {
    for _ in 0..frames {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![move_forward_command()]);
        app.update();
    }
}

#[test]
fn controller_climbs_amnesia_style_step_run() {
    let mut app = app();
    let config = FirstPersonControllerConfig {
        ground_accel: 100.0,
        side_accel: 100.0,
        step_check_interval: 0.0,
        ..default()
    };
    let start_y = config.height(ControllerStance::Standing) * 0.5;
    let player = spawn_player(&mut app, &config, Vec3::new(0.0, start_y, 3.0));
    spawn_static_box(
        &mut app,
        Vec3::new(10.0, 0.2, 16.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    for step in 0..4 {
        let height = 0.12 * (step as f32 + 1.0);
        spawn_static_box(
            &mut app,
            Vec3::new(2.0, height, 0.55),
            Transform::from_xyz(0.0, height * 0.5, 1.6 - step as f32 * 0.55),
        );
    }

    app.update();
    run_forward(&mut app, 45);

    let transform = app.world().get::<Transform>(player).unwrap();
    assert!(
        transform.translation.z < 0.6,
        "did not reach stair run: {transform:?}"
    );
    assert!(
        transform.translation.y > start_y + 0.3,
        "did not climb stairs: {transform:?}"
    );
}

#[test]
fn default_controller_climbs_low_stair_without_sprint() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let start_y = config.height(ControllerStance::Standing) * 0.5;
    let player = spawn_player(&mut app, &config, Vec3::new(0.0, start_y, 2.0));
    spawn_static_box(
        &mut app,
        Vec3::new(10.0, 0.2, 10.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(2.0, 0.12, 0.55),
        Transform::from_xyz(0.0, 0.06, 0.6),
    );

    app.update();
    let mut max_y = start_y;
    for _ in 0..60 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![move_forward_command()]);
        app.update();
        max_y = max_y.max(app.world().get::<Transform>(player).unwrap().translation.y);
    }

    let transform = app.world().get::<Transform>(player).unwrap();
    assert!(
        max_y > start_y + 0.08 && transform.translation.z < 0.45,
        "default walk did not climb low stair: max_y={max_y}, transform={transform:?}"
    );
}

#[test]
fn default_controller_climbs_low_stair_run_without_sprint() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let start_y = config.height(ControllerStance::Standing) * 0.5;
    let player = spawn_player(&mut app, &config, Vec3::new(0.0, start_y, 3.0));
    spawn_static_box(
        &mut app,
        Vec3::new(10.0, 0.2, 16.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    for step in 0..4 {
        let height = 0.12 * (step as f32 + 1.0);
        spawn_static_box(
            &mut app,
            Vec3::new(2.0, height, 0.55),
            Transform::from_xyz(0.0, height * 0.5, 1.6 - step as f32 * 0.55),
        );
    }

    app.update();
    let mut max_y = start_y;
    for _ in 0..90 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![move_forward_command()]);
        app.update();
        max_y = max_y.max(app.world().get::<Transform>(player).unwrap().translation.y);
    }

    let transform = app.world().get::<Transform>(player).unwrap();
    assert!(
        transform.translation.z < 0.4 && max_y > start_y + 0.35,
        "default walk did not climb low stair run: max_y={max_y}, transform={transform:?}"
    );
}

#[test]
fn step_climbing_latches_grounding_like_hpl2() {
    let mut app = app();
    let config = FirstPersonControllerConfig {
        ground_accel: 100.0,
        side_accel: 100.0,
        step_check_interval: 0.0,
        ..default()
    };
    let start_y = config.height(ControllerStance::Standing) * 0.5;
    let player = spawn_player(&mut app, &config, Vec3::new(0.0, start_y, 1.0));
    spawn_static_box(
        &mut app,
        Vec3::new(10.0, 0.2, 10.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(2.0, 0.12, 0.55),
        Transform::from_xyz(0.0, 0.06, 0.2),
    );

    app.update();
    for _ in 0..12 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![move_forward_command()]);
        app.update();
        if app
            .world()
            .get::<FirstPersonMotorState>(player)
            .unwrap()
            .climbing
        {
            break;
        }
    }

    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    assert!(state.climbing, "step climb did not latch: {state:?}");
    assert!(state.grounded);
    assert_eq!(state.ground_contact_ticks, config.ground_sticky_ticks);
}

#[test]
fn stair_attempt_is_not_blocked_by_upward_force_velocity_like_hpl2() {
    let mut app = app();
    let config = FirstPersonControllerConfig {
        ground_accel: 100.0,
        side_accel: 100.0,
        step_check_interval: 0.0,
        ..default()
    };
    let start_y = config.height(ControllerStance::Standing) * 0.5;
    let player = spawn_player(&mut app, &config, Vec3::new(0.0, start_y, 0.84));
    spawn_static_box(
        &mut app,
        Vec3::new(10.0, 0.2, 10.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(2.0, 0.12, 0.55),
        Transform::from_xyz(0.0, 0.06, 0.2),
    );

    app.update();
    {
        let mut state = app
            .world_mut()
            .get_mut::<FirstPersonMotorState>(player)
            .unwrap();
        state.velocity.y = 0.25;
        state.grounded = true;
        state.ground_contact_ticks = config.ground_sticky_ticks;
    }
    app.world_mut()
        .resource_mut::<PlayerCommandQueue>()
        .replace(vec![move_forward_command()]);
    app.update();

    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    assert!(
        state.climbing,
        "upward force velocity blocked step climb: {state:?}"
    );
}

#[test]
fn default_step_and_grounding_values_follow_hpl2_constructor() {
    let config = FirstPersonControllerConfig::default();

    assert_eq!(config.ground_sticky_ticks, 12);
    assert_eq!(config.step_climb_speed, 1.0);
    assert_eq!(
        config.max_step_height,
        config.height(ControllerStance::Standing) * 0.2
    );
    assert_eq!(config.max_step_height_in_air, config.max_step_height);
    assert!(!config.accurate_climbing);
    assert_eq!(config.climb_forward_mul, 1.0);
    assert_eq!(config.skin_width, 0.0);
}

#[test]
fn controller_moves_smoothly_up_shallow_slope() {
    let mut app = app();
    let config = FirstPersonControllerConfig {
        ground_accel: 100.0,
        side_accel: 100.0,
        ..default()
    };
    let start_y = config.height(ControllerStance::Standing) * 0.5 + 0.15;
    let player = spawn_player(&mut app, &config, Vec3::new(0.0, start_y, 3.0));
    spawn_static_box(
        &mut app,
        Vec3::new(2.4, 0.35, 5.0),
        Transform::from_xyz(0.0, 0.35, 0.0)
            .with_rotation(Quat::from_rotation_x(12.0_f32.to_radians())),
    );

    app.update();
    let mut last_z = app.world().get::<Transform>(player).unwrap().translation.z;
    let mut z_backtracks = 0;
    for _ in 0..90 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![move_forward_command()]);
        app.update();
        let z = app.world().get::<Transform>(player).unwrap().translation.z;
        if z > last_z + 0.001 {
            z_backtracks += 1;
        }
        last_z = z;
    }

    let transform = app.world().get::<Transform>(player).unwrap();
    assert!(
        transform.translation.z < -0.5,
        "did not climb slope: {transform:?}"
    );
    assert!(
        z_backtracks < 3,
        "slope motion jittered/backtracked {z_backtracks} times"
    );
}

#[test]
fn airborne_ground_probe_does_not_snap_player_down_before_collision() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let player = spawn_player(
        &mut app,
        &config,
        Vec3::new(0.0, half_height + config.ground_probe_distance * 0.5, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(10.0, 0.2, 10.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );

    app.update();

    let transform = app.world().get::<Transform>(player).unwrap();
    assert!(
        transform.translation.y > half_height + config.ground_probe_distance * 0.25,
        "airborne player was magnet-snapped to ground: {transform:?}"
    );
}

#[test]
fn stair_edge_idle_does_not_micro_jump() {
    let mut app = app();
    let config = FirstPersonControllerConfig {
        ground_accel: 100.0,
        side_accel: 100.0,
        step_check_interval: 0.0,
        ..default()
    };
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let player = spawn_player(&mut app, &config, Vec3::new(0.0, half_height + 0.24, 0.32));
    spawn_static_box(
        &mut app,
        Vec3::new(10.0, 0.2, 10.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(2.0, 0.24, 1.0),
        Transform::from_xyz(0.0, 0.12, 0.0),
    );

    app.update();
    let start_y = app.world().get::<Transform>(player).unwrap().translation.y;
    for _ in 0..90 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(Vec::new());
        app.update();
    }

    let transform = app.world().get::<Transform>(player).unwrap();
    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    assert!(
        (transform.translation.y - start_y).abs() < 0.002,
        "idle stair edge micro-jumped: start_y={start_y}, transform={transform:?}, state={state:?}"
    );
    assert!(!state.climbing);
}

#[test]
fn moving_along_stair_edge_does_not_repeatedly_pop_up() {
    let mut app = app();
    let config = FirstPersonControllerConfig {
        ground_accel: 100.0,
        side_accel: 100.0,
        step_check_interval: 0.0,
        ..default()
    };
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let player = spawn_player(&mut app, &config, Vec3::new(-0.8, half_height + 0.24, 0.32));
    spawn_static_box(
        &mut app,
        Vec3::new(10.0, 0.2, 10.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(2.0, 0.24, 1.0),
        Transform::from_xyz(0.0, 0.12, 0.0),
    );

    app.update();
    let start_y = app.world().get::<Transform>(player).unwrap().translation.y;
    let mut upward_pops = 0;
    let mut last_y = start_y;
    for _ in 0..90 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![move_right_command()]);
        app.update();
        let y = app.world().get::<Transform>(player).unwrap().translation.y;
        if y > last_y + 0.01 {
            upward_pops += 1;
        }
        last_y = y;
    }

    let transform = app.world().get::<Transform>(player).unwrap();
    assert_eq!(
        upward_pops, 0,
        "moving along stair edge popped up: start_y={start_y}, transform={transform:?}"
    );
}
