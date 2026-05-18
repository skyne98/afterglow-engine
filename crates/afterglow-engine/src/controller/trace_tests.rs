use super::*;
use crate::{
    core::AfterglowCorePlugin,
    physics::{AfterglowPhysicsPlugin, PhysicsBody, PhysicsCollider},
};
use bevy::time::TimeUpdateStrategy;
use leafwing_input_manager::action_state::ActionState;
use std::time::Duration;

fn app_with_trace(max_frames: usize) -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AfterglowCorePlugin,
        AfterglowPhysicsPlugin,
        AfterglowFirstPersonControllerPlugin,
    ));
    app.insert_resource(FirstPersonControllerTrace::enabled(max_frames));
    app.finish();
    app.cleanup();
    *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
        TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 60.0));
    app
}

fn move_forward_command() -> ActionState<crate::input::AfterglowAction> {
    test_input::command(&[("move.y", 1.0)], &[])
}

fn spawn_player(app: &mut App, config: FirstPersonControllerConfig, position: Vec3) -> Entity {
    app.world_mut()
        .spawn((
            FirstPersonController { config },
            Transform::from_translation(position),
        ))
        .id()
}

fn spawn_camera(app: &mut App, target: Entity) -> Entity {
    app.world_mut()
        .spawn((FirstPersonCameraRig::new(target), Transform::default()))
        .id()
}

fn spawn_static_box(app: &mut App, size: Vec3, translation: Vec3) {
    app.world_mut().spawn((
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(size),
        Transform::from_translation(translation),
    ));
}

fn push_command(app: &mut App, player: Entity) {
    test_input::set_input(app, player, move_forward_command());
}

#[test]
fn controller_trace_records_the_exact_collision_phase() {
    let mut app = app_with_trace(512);
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let player = spawn_player(&mut app, config, Vec3::new(0.0, half_height, 1.2));
    let camera = spawn_camera(&mut app, player);
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 8.0),
        Vec3::new(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(2.0, 0.7, 0.2),
        Vec3::new(0.0, 0.35, -0.4),
    );

    app.update();
    for _ in 1..90 {
        push_command(&mut app, player);
        app.update();
    }

    let trace = app.world().resource::<FirstPersonControllerTrace>();
    let blocked = trace
        .controller_frames
        .iter()
        .find(|frame| frame.entity == player && frame.horizontal_pushback.length() > 0.0001)
        .expect("walking into the blocker should record a horizontal depenetration");
    let expected_after_horizontal = blocked.after_stance_position
        + blocked.intended_horizontal_delta
        + blocked.horizontal_pushback;
    assert!(
        blocked
            .after_horizontal_position
            .distance(expected_after_horizontal)
            < 0.001,
        "trace phase positions must reconstruct horizontal collision exactly: {blocked:?}"
    );
    let step_rejection = trace
        .controller_frames
        .iter()
        .find(|frame| {
            frame.entity == player
                && frame.step.rays.iter().any(|ray| {
                    ray.reject_reason == FirstPersonStepRejectReason::TooHigh
                        || ray.reject_reason == FirstPersonStepRejectReason::ShapeBlocked
                })
        })
        .expect("too-high obstacle should record a stair rejection near the collision");
    assert!(
        step_rejection.step.ran,
        "stair rejection should come from a real raycast attempt: {step_rejection:?}"
    );

    let camera_frame = trace
        .camera_frames
        .iter()
        .rev()
        .find(|frame| frame.camera == camera && frame.target == player)
        .expect("camera trace should record the presentation layer");
    assert!(
        camera_frame
            .final_position
            .distance(camera_frame.base_position + camera_frame.bob_offset)
            < 0.0001,
        "camera trace must split body follow from bob/landing offset: {camera_frame:?}"
    );
}

#[test]
fn step_trace_records_successful_ray_climbs() {
    let mut app = app_with_trace(512);
    let config = FirstPersonControllerConfig {
        accurate_climbing: true,
        ..default()
    };
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let player = spawn_player(&mut app, config.clone(), Vec3::new(0.0, half_height, 1.4));
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 10.0),
        Vec3::new(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(2.0, 0.12, 10.0),
        Vec3::new(0.0, 0.06, -4.5),
    );

    app.update();
    for _ in 1..120 {
        push_command(&mut app, player);
        app.update();
    }

    let trace = app.world().resource::<FirstPersonControllerTrace>();
    let accepted = trace
        .controller_frames
        .iter()
        .find(|frame| frame.entity == player && frame.step.accepted)
        .expect("low ledge should produce an accepted step trace");
    assert_eq!(accepted.step.ray_count, 3);
    assert!(
        (accepted.step.lift - config.step_climb_speed / 60.0).abs() < 0.0001,
        "step trace should expose the actual per-frame lift"
    );
    assert!(
        accepted
            .step
            .rays
            .iter()
            .any(|ray| ray.reject_reason == FirstPersonStepRejectReason::Accepted),
        "accepted trace must preserve the winning ray: {accepted:?}"
    );
}

#[test]
fn trace_resource_keeps_a_bounded_ring_of_recent_frames() {
    let mut app = app_with_trace(3);
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let player = spawn_player(&mut app, config, Vec3::new(0.0, half_height, 1.2));
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 8.0),
        Vec3::new(0.0, -0.1, 0.0),
    );

    app.update();
    for _ in 1..8 {
        push_command(&mut app, player);
        app.update();
    }

    let trace = app.world().resource::<FirstPersonControllerTrace>();
    assert_eq!(trace.controller_frames.len(), 3);
    assert_eq!(trace.controller_frames.first().unwrap().tick, 0);
    assert_eq!(trace.controller_frames.last().unwrap().tick, 0);
}
