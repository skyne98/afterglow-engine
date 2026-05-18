use super::*;
use crate::{
    core::AfterglowCorePlugin,
    physics::{AfterglowPhysicsPlugin, PhysicsBody, PhysicsCollider},
};
use bevy::time::TimeUpdateStrategy;
use leafwing_input_manager::action_state::ActionState;
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
struct StairSample {
    position: Vec3,
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

fn sample(app: &App, player: Entity) -> StairSample {
    StairSample {
        position: app.world().get::<Transform>(player).unwrap().translation,
    }
}

#[test]
fn controller_keeps_forward_progress_through_low_stair_contact() {
    let mut app = app();
    let config = FirstPersonControllerConfig {
        ground_accel: 100.0,
        side_accel: 100.0,
        step_check_interval: 0.0,
        ..default()
    };
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let step_height = 0.12;
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
        Vec3::new(2.0, step_height, 0.55),
        Transform::from_xyz(0.0, step_height * 0.5, 0.2),
    );

    app.update();
    let mut samples = Vec::with_capacity(61);
    samples.push(sample(&app, player));
    for _ in 0..60 {
        test_input::set_input(&mut app, player, move_forward_command());
        app.update();
        samples.push(sample(&app, player));
    }

    let min_expected_progress = config.ground_speed / 60.0 * 0.35;
    let mut checked_contact_frames = 0;
    for pair in samples.windows(2) {
        let previous = pair[0];
        let current = pair[1];
        let in_contact_window = current.position.z < 1.0
            && current.position.z > 0.35
            && current.position.y < half_height + step_height + 0.05;
        if !in_contact_window {
            continue;
        }
        checked_contact_frames += 1;
        let forward_progress = previous.position.z - current.position.z;
        assert!(
            forward_progress >= min_expected_progress,
            "low stair contact stalled forward progress: progress={forward_progress}, expected={min_expected_progress}, samples={samples:?}"
        );
    }
    assert!(
        checked_contact_frames > 0,
        "test never sampled stair contact window: {samples:?}"
    );
}
