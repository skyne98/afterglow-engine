use super::*;
use crate::{
    core::AfterglowCorePlugin,
    input::AfterglowAction,
    physics::{AfterglowPhysicsPlugin, PhysicsBody, PhysicsCollider},
};
use bevy::time::TimeUpdateStrategy;
use leafwing_input_manager::action_state::ActionState;
use std::time::Duration;

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

fn crouch_forward_command() -> ActionState<AfterglowAction> {
    crouch_move_command("move.y", 1.0)
}

fn crouch_right_command() -> ActionState<AfterglowAction> {
    crouch_move_command("move.x", 1.0)
}

fn crouch_move_command(axis: &'static str, value: f32) -> ActionState<AfterglowAction> {
    test_input::command(&[(axis, value)], &[AfterglowAction::Crouch])
}

fn spawn_crouched_player(app: &mut App, config: &FirstPersonControllerConfig, pos: Vec3) -> Entity {
    let standing_center = pos
        + Vec3::Y
            * (config.height(ControllerStance::Standing)
                - config.height(ControllerStance::Crouching))
            * 0.5;
    app.world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_translation(standing_center),
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

fn hold_crouch(app: &mut App, player: Entity) {
    test_input::set_input(
        app,
        player,
        test_input::command(&[], &[AfterglowAction::Crouch]),
    );
}

fn update_holding_crouch(app: &mut App, player: Entity) {
    hold_crouch(app, player);
    app.update();
}

fn settle_crouch(app: &mut App, player: Entity) {
    for _ in 0..90 {
        update_holding_crouch(app, player);
    }
}

#[test]
fn crouched_controller_climbs_low_stair_fully() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let start_y = config.height(ControllerStance::Crouching) * 0.5;
    let player = spawn_crouched_player(&mut app, &config, Vec3::new(0.0, start_y, 2.0));
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

    settle_crouch(&mut app, player);
    let mut max_y = start_y;
    for _ in 0..120 {
        test_input::set_input(&mut app, player, crouch_forward_command());
        app.update();
        max_y = max_y.max(app.world().get::<Transform>(player).unwrap().translation.y);
    }

    let transform = app.world().get::<Transform>(player).unwrap();
    assert!(
        max_y > start_y + 0.1 && transform.translation.z < 0.35,
        "crouched walk did not fully climb low stair: max_y={max_y}, transform={transform:?}"
    );
}

#[test]
fn crouched_controller_climbs_low_stair_run_fully() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let start_y = config.height(ControllerStance::Crouching) * 0.5;
    let player = spawn_crouched_player(&mut app, &config, Vec3::new(0.0, start_y, 3.0));
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

    settle_crouch(&mut app, player);
    let mut max_y = start_y;
    let mut reached_top = false;
    for _ in 0..180 {
        test_input::set_input(&mut app, player, crouch_forward_command());
        app.update();
        let transform = app.world().get::<Transform>(player).unwrap();
        max_y = max_y.max(transform.translation.y);
        reached_top |= transform.translation.z < 0.45 && transform.translation.y > start_y + 0.35;
    }
    let transform = app.world().get::<Transform>(player).unwrap();
    assert!(
        reached_top,
        "crouched walk did not fully climb low stair run: max_y={max_y}, final={transform:?}"
    );
}

#[test]
fn crouched_controller_finishes_step_when_horizontal_move_is_pinned() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let start_y = config.height(ControllerStance::Crouching) * 0.5;
    let player = spawn_crouched_player(&mut app, &config, Vec3::new(0.0, start_y, 3.125));
    spawn_static_box(
        &mut app,
        Vec3::new(10.0, 0.2, 10.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(2.0, 0.12, 0.55),
        Transform::from_xyz(0.0, 0.06, 2.725),
    );

    settle_crouch(&mut app, player);
    let target_y = start_y + 0.12;
    let mut reached_step_top = false;
    let mut crossed_riser = false;
    for _ in 0..90 {
        test_input::set_input(&mut app, player, crouch_forward_command());
        app.update();
        let transform = app.world().get::<Transform>(player).unwrap();
        reached_step_top |= transform.translation.y >= target_y - 0.005;
        crossed_riser |= transform.translation.z < 3.0;
    }

    let transform = app.world().get::<Transform>(player).unwrap();
    assert!(
        reached_step_top && crossed_riser,
        "pinned crouched step climb did not finish: target_y={target_y}, final={transform:?}"
    );
}

#[test]
fn crouched_climb_releasing_move_input_clears_climbing() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let start_y = config.height(ControllerStance::Crouching) * 0.5;
    let player = spawn_crouched_player(&mut app, &config, Vec3::new(0.0, start_y, 0.0));
    spawn_static_box(
        &mut app,
        Vec3::new(10.0, 0.2, 10.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );

    settle_crouch(&mut app, player);
    {
        let mut state = app
            .world_mut()
            .get_mut::<FirstPersonMotorState>(player)
            .unwrap();
        state.climbing = true;
        state.grounded = true;
        state.ground_contact_ticks = config.ground_sticky_ticks;
    }

    update_holding_crouch(&mut app, player);

    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    assert!(
        !state.climbing,
        "climbing stayed latched after releasing move input: {state:?}"
    );
}

#[test]
fn crouched_step_climbing_latches_grounding_like_hpl2() {
    let mut app = app();
    let config = FirstPersonControllerConfig {
        ground_accel: 100.0,
        side_accel: 100.0,
        step_check_interval: 0.0,
        ..default()
    };
    let start_y = config.height(ControllerStance::Crouching) * 0.5;
    let player = spawn_crouched_player(&mut app, &config, Vec3::new(0.0, start_y, 1.0));
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

    settle_crouch(&mut app, player);
    for _ in 0..12 {
        test_input::set_input(&mut app, player, crouch_forward_command());
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
    assert!(
        state.climbing,
        "crouched step climb did not latch: {state:?}"
    );
    assert!(state.grounded);
    assert_eq!(state.ground_contact_ticks, config.ground_sticky_ticks);
}

#[test]
fn crouched_stair_attempt_is_not_blocked_by_upward_force_velocity_like_hpl2() {
    let mut app = app();
    let config = FirstPersonControllerConfig {
        ground_accel: 100.0,
        side_accel: 100.0,
        step_check_interval: 0.0,
        ..default()
    };
    let start_y = config.height(ControllerStance::Crouching) * 0.5;
    let player = spawn_crouched_player(&mut app, &config, Vec3::new(0.0, start_y, 0.84));
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

    settle_crouch(&mut app, player);
    {
        let mut state = app
            .world_mut()
            .get_mut::<FirstPersonMotorState>(player)
            .unwrap();
        state.velocity.y = 0.25;
        state.grounded = true;
        state.ground_contact_ticks = config.ground_sticky_ticks;
    }
    test_input::set_input(&mut app, player, crouch_forward_command());
    app.update();

    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    assert!(
        state.climbing,
        "upward force velocity blocked crouched step climb: {state:?}"
    );
}

#[test]
fn crouched_stair_edge_idle_does_not_micro_jump() {
    let mut app = app();
    let config = FirstPersonControllerConfig {
        ground_accel: 100.0,
        side_accel: 100.0,
        step_check_interval: 0.0,
        ..default()
    };
    let half_height = config.height(ControllerStance::Crouching) * 0.5;
    let player = spawn_crouched_player(&mut app, &config, Vec3::new(0.0, half_height + 0.24, 0.32));
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

    settle_crouch(&mut app, player);
    let start_y = app.world().get::<Transform>(player).unwrap().translation.y;
    for _ in 0..90 {
        update_holding_crouch(&mut app, player);
    }

    let transform = app.world().get::<Transform>(player).unwrap();
    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    assert!(
        (transform.translation.y - start_y).abs() < 0.002,
        "idle crouched stair edge micro-jumped: start_y={start_y}, transform={transform:?}, state={state:?}"
    );
    assert!(!state.climbing);
}

#[test]
fn crouched_moving_along_stair_edge_does_not_repeatedly_pop_up() {
    let mut app = app();
    let config = FirstPersonControllerConfig {
        ground_accel: 100.0,
        side_accel: 100.0,
        step_check_interval: 0.0,
        ..default()
    };
    let half_height = config.height(ControllerStance::Crouching) * 0.5;
    let player =
        spawn_crouched_player(&mut app, &config, Vec3::new(-0.8, half_height + 0.24, 0.32));
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

    settle_crouch(&mut app, player);
    let start_y = app.world().get::<Transform>(player).unwrap().translation.y;
    let mut upward_pops = 0;
    let mut last_y = start_y;
    for _ in 0..90 {
        test_input::set_input(&mut app, player, crouch_right_command());
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
        "moving crouched along stair edge popped up: start_y={start_y}, transform={transform:?}"
    );
}

#[test]
fn crouched_controller_descends_low_step_without_hanging() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Crouching) * 0.5;
    let player = spawn_crouched_player(&mut app, &config, Vec3::new(0.0, half_height + 0.12, 0.6));
    spawn_static_box(
        &mut app,
        Vec3::new(10.0, 0.2, 10.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(2.0, 0.12, 1.0),
        Transform::from_xyz(0.0, 0.06, 0.8),
    );

    settle_crouch(&mut app, player);
    let mut lowest_y = f32::MAX;
    for _ in 0..120 {
        test_input::set_input(&mut app, player, crouch_forward_command());
        app.update();
        lowest_y = lowest_y.min(app.world().get::<Transform>(player).unwrap().translation.y);
    }

    let transform = app.world().get::<Transform>(player).unwrap();
    assert!(
        transform.translation.z < -0.4 && lowest_y <= half_height + 0.01,
        "crouched descent hung on the step: lowest_y={lowest_y}, transform={transform:?}"
    );
}

#[test]
fn crouched_controller_rejects_step_above_allowed_height() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Crouching) * 0.5;
    let player = spawn_crouched_player(&mut app, &config, Vec3::new(0.0, half_height, 1.0));
    spawn_static_box(
        &mut app,
        Vec3::new(10.0, 0.2, 10.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    let obstacle_height = config.max_step_height + 0.06;
    spawn_static_box(
        &mut app,
        Vec3::new(2.0, obstacle_height, 0.55),
        Transform::from_xyz(0.0, obstacle_height * 0.5, 0.2),
    );

    settle_crouch(&mut app, player);
    let mut max_y = half_height;
    for _ in 0..90 {
        test_input::set_input(&mut app, player, crouch_forward_command());
        app.update();
        max_y = max_y.max(app.world().get::<Transform>(player).unwrap().translation.y);
    }

    let transform = app.world().get::<Transform>(player).unwrap();
    assert!(
        max_y < half_height + config.max_step_height * 0.5 && transform.translation.z > 0.5,
        "crouched controller climbed an over-height step: max_y={max_y}, transform={transform:?}"
    );
}
