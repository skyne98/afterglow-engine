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
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
        seconds,
    )));
    app.finish();
    app.cleanup();
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
fn look_updates_on_render_frame_without_fixed_tick() {
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
    app.update();

    *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
        TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 240.0));
    test_input::set_input(
        &mut app,
        player,
        test_input::command(&[("look.x", 12.0)], &[]),
    );
    app.update();

    let motor = app.world().get::<FirstPersonMotorState>(player).unwrap();
    assert!(
        motor.yaw < -0.001,
        "look input should update yaw before the next fixed tick: {motor:?}"
    );
}

#[test]
fn walking_camera_advances_between_fixed_ticks_at_render_rate() {
    let mut app = app_with_dt(1.0 / 120.0);
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
    let camera = app
        .world_mut()
        .spawn((
            FirstPersonCameraRig {
                target: player,
                config: FirstPersonCameraConfig {
                    walk_bob_amplitude: Vec2::ZERO,
                    run_bob_amplitude: Vec2::ZERO,
                    crouch_bob_amplitude: Vec2::ZERO,
                    ..default()
                },
            },
            Transform::default(),
        ))
        .id();
    spawn_floor(&mut app);

    for _ in 0..120 {
        test_input::set_input(
            &mut app,
            player,
            test_input::command(&[("move.y", 1.0)], &[]),
        );
        app.update();
    }

    let mut positions = Vec::with_capacity(24);
    for _ in 0..24 {
        test_input::set_input(
            &mut app,
            player,
            test_input::command(&[("move.y", 1.0)], &[]),
        );
        app.update();
        positions.push(app.world().get::<Transform>(camera).unwrap().translation);
    }

    let stalled_frames = positions
        .windows(2)
        .filter(|pair| (pair[1].z - pair[0].z).abs() < 0.0001)
        .count();
    assert_eq!(
        stalled_frames, 0,
        "camera presentation should advance every render frame, not only fixed ticks: {positions:?}"
    );
}
