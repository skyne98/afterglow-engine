use super::*;
use crate::{
    controller::{AfterglowFirstPersonControllerPlugin, FirstPersonControllerConfig},
    core::AfterglowCorePlugin,
    input::{InputActionValue, InputAxis, InputAxisValue, PlayerCommand, PlayerCommandQueue},
    network::NetworkPlayerId,
    physics::{AfterglowPhysicsPlugin, PhysicsBody, PhysicsCollider},
};
use bevy::time::TimeUpdateStrategy;
use std::time::Duration;

#[test]
fn flat_left_strafe_camera_trace_has_no_repeating_pattern_without_bob() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AfterglowCorePlugin,
        AfterglowPhysicsPlugin,
        AfterglowFirstPersonControllerPlugin,
    ));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
        1.0 / 60.0,
    )));
    app.init_resource::<PlayerCommandQueue>();
    app.finish();
    app.cleanup();

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
    let camera_config = FirstPersonCameraConfig {
        walk_bob_amplitude: Vec2::ZERO,
        run_bob_amplitude: Vec2::ZERO,
        crouch_bob_amplitude: Vec2::ZERO,
        ..default()
    };
    let camera = app
        .world_mut()
        .spawn((
            FirstPersonCameraRig {
                target: player,
                config: camera_config,
            },
            Transform::default(),
        ))
        .id();
    app.world_mut().spawn((
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(Vec3::new(200.0, 0.2, 200.0)),
        Transform::from_xyz(0.0, -0.1, 0.0),
    ));

    for _ in 0..180 {
        app.update();
    }

    let mut trace = Vec::with_capacity(180);
    for tick in 0..180 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![PlayerCommand {
                player: NetworkPlayerId(1),
                tick,
                axes: vec![InputAxisValue {
                    axis: InputAxis::new("move.x"),
                    value: -1.0,
                }],
                ..default()
            }]);
        app.update();
        trace.push(app.world().get::<Transform>(camera).unwrap().translation);
    }

    let settled = &trace[60..];
    let y_range = axis_range(settled, |position| position.y);
    let z_range = axis_range(settled, |position| position.z);
    let x_inversions = derivative_sign_inversions(settled, |position| position.x);
    let y_repeats = periodic_derivative_score(settled, |position| position.y, 30);
    let z_repeats = periodic_derivative_score(settled, |position| position.z, 30);

    assert!(
        y_range < 0.0005
            && z_range < 0.0005
            && x_inversions == 0
            && y_repeats < 0.00001
            && z_repeats < 0.00001,
        "flat left strafe camera trace should be smooth\nx:\n{}\ny:\n{}\nz:\n{}\ny_range={y_range:.6} z_range={z_range:.6} x_inversions={x_inversions} y_repeats={y_repeats:.6} z_repeats={z_repeats:.6}",
        ascii_graph(settled, |position| position.x),
        ascii_graph(settled, |position| position.y),
        ascii_graph(settled, |position| position.z),
    );
}

#[test]
fn low_blocker_camera_trace_stays_still_when_body_is_blocked() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AfterglowCorePlugin,
        AfterglowPhysicsPlugin,
        AfterglowFirstPersonControllerPlugin,
    ));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
        1.0 / 60.0,
    )));
    app.init_resource::<PlayerCommandQueue>();
    app.finish();
    app.cleanup();

    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
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
    let camera = app
        .world_mut()
        .spawn((FirstPersonCameraRig::new(player), Transform::default()))
        .id();
    app.world_mut().spawn((
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(Vec3::new(8.0, 0.2, 10.0)),
        Transform::from_xyz(0.0, -0.1, 0.0),
    ));
    let obstacle_height = config.max_step_height + 0.14;
    app.world_mut().spawn((
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(Vec3::new(1.2, obstacle_height, 0.55)),
        Transform::from_xyz(0.0, obstacle_height * 0.5, 0.2),
    ));

    let mut trace = Vec::with_capacity(120);
    for tick in 0..120 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![PlayerCommand {
                player: NetworkPlayerId(1),
                tick,
                axes: vec![InputAxisValue {
                    axis: InputAxis::new("move.y"),
                    value: 1.0,
                }],
                ..default()
            }]);
        app.update();
        trace.push(app.world().get::<Transform>(camera).unwrap().translation);
    }

    let settled = &trace[60..];
    let x_range = axis_range(settled, |position| position.x);
    let y_range = axis_range(settled, |position| position.y);
    let z_range = axis_range(settled, |position| position.z);
    assert!(
        x_range < 0.004 && y_range < 0.004 && z_range < 0.008,
        "blocked low obstacle should not drive camera bob or jitter\nx:\n{}\ny:\n{}\nz:\n{}\nx_range={x_range:.6} y_range={y_range:.6} z_range={z_range:.6}",
        ascii_graph(settled, |position| position.x),
        ascii_graph(settled, |position| position.y),
        ascii_graph(settled, |position| position.z),
    );
}

#[test]
fn stair_step_up_camera_trace_smooths_authoritative_landing_snap() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AfterglowCorePlugin,
        AfterglowPhysicsPlugin,
        AfterglowFirstPersonControllerPlugin,
    ));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
        1.0 / 60.0,
    )));
    app.init_resource::<PlayerCommandQueue>();
    app.finish();
    app.cleanup();

    let config = FirstPersonControllerConfig {
        ground_accel: 100.0,
        side_accel: 100.0,
        step_check_interval: 0.0,
        ..default()
    };
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                player: NetworkPlayerId(1),
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, 1.4),
        ))
        .id();
    let camera_config = FirstPersonCameraConfig {
        walk_bob_amplitude: Vec2::ZERO,
        run_bob_amplitude: Vec2::ZERO,
        crouch_bob_amplitude: Vec2::ZERO,
        ..default()
    };
    let camera = app
        .world_mut()
        .spawn((
            FirstPersonCameraRig {
                target: player,
                config: camera_config,
            },
            Transform::default(),
        ))
        .id();
    app.world_mut().spawn((
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(Vec3::new(8.0, 0.2, 10.0)),
        Transform::from_xyz(0.0, -0.1, 0.0),
    ));
    app.world_mut().spawn((
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(Vec3::new(2.0, 0.12, 0.55)),
        Transform::from_xyz(0.0, 0.06, 0.2),
    ));

    let mut body_trace = Vec::with_capacity(90);
    let mut camera_trace = Vec::with_capacity(90);
    for tick in 0..90 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![PlayerCommand {
                player: NetworkPlayerId(1),
                tick,
                axes: vec![InputAxisValue {
                    axis: InputAxis::new("move.y"),
                    value: 1.0,
                }],
                ..default()
            }]);
        app.update();
        body_trace.push(app.world().get::<Transform>(player).unwrap().translation);
        camera_trace.push(app.world().get::<Transform>(camera).unwrap().translation);
    }

    let body_snap = max_positive_y_delta(&body_trace);
    let camera_snap = max_positive_y_delta(&camera_trace);
    assert!(
        body_snap > 0.08,
        "test never observed body step-up: {body_trace:?}"
    );
    assert!(
        camera_snap < body_snap * 0.75 && camera_snap < 0.08,
        "camera did not smooth stair step-up\nbody_snap={body_snap:.6} camera_snap={camera_snap:.6}\nbody:\n{}\ncamera:\n{}",
        ascii_graph(&body_trace, |position| position.y),
        ascii_graph(&camera_trace, |position| position.y),
    );
}

#[test]
fn low_blocker_camera_trace_stays_still_after_sprint_reset() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AfterglowCorePlugin,
        AfterglowPhysicsPlugin,
        AfterglowFirstPersonControllerPlugin,
    ));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
        1.0 / 60.0,
    )));
    app.init_resource::<PlayerCommandQueue>();
    app.finish();
    app.cleanup();

    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
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
    let camera = app
        .world_mut()
        .spawn((FirstPersonCameraRig::new(player), Transform::default()))
        .id();
    app.world_mut().spawn((
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(Vec3::new(8.0, 0.2, 14.0)),
        Transform::from_xyz(0.0, -0.1, 0.0),
    ));
    let obstacle_height = config.max_step_height + 0.14;
    app.world_mut().spawn((
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(Vec3::new(1.2, obstacle_height, 0.55)),
        Transform::from_xyz(0.0, obstacle_height * 0.5, 0.2),
    ));

    let mut tick = 0;
    for _ in 0..24 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![PlayerCommand {
                player: NetworkPlayerId(1),
                tick,
                axes: vec![InputAxisValue {
                    axis: InputAxis::new("move.y"),
                    value: 1.0,
                }],
                actions: vec![InputActionValue::held("sprint")],
                ..default()
            }]);
        tick += 1;
        app.update();
    }
    for _ in 0..60 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(Vec::new());
        tick += 1;
        app.update();
    }
    let motor = app.world().get::<FirstPersonMotorState>(player).unwrap();
    let camera_state = app.world().get::<FirstPersonCameraState>(camera).unwrap();
    assert!(motor.forward_speed.abs() < 0.01);
    assert!(!camera_state.bobbing);
    assert!(camera_state.current_bob_amplitude.length() < 0.001);
    assert!((camera_state.fov - FirstPersonCameraConfig::default().fov).abs() < 0.002);

    let mut trace = Vec::with_capacity(120);
    for _ in 0..120 {
        app.world_mut()
            .resource_mut::<PlayerCommandQueue>()
            .replace(vec![PlayerCommand {
                player: NetworkPlayerId(1),
                tick,
                axes: vec![InputAxisValue {
                    axis: InputAxis::new("move.y"),
                    value: 1.0,
                }],
                ..default()
            }]);
        tick += 1;
        app.update();
        trace.push(app.world().get::<Transform>(camera).unwrap().translation);
    }

    let settled = &trace[60..];
    let x_range = axis_range(settled, |position| position.x);
    let y_range = axis_range(settled, |position| position.y);
    let z_range = axis_range(settled, |position| position.z);
    assert!(
        x_range < 0.004 && y_range < 0.004 && z_range < 0.008,
        "blocked low obstacle should not restart camera bob after sprint\nx:\n{}\ny:\n{}\nz:\n{}\nx_range={x_range:.6} y_range={y_range:.6} z_range={z_range:.6}",
        ascii_graph(settled, |position| position.x),
        ascii_graph(settled, |position| position.y),
        ascii_graph(settled, |position| position.z),
    );
}

fn axis_range(samples: &[Vec3], axis: impl Fn(Vec3) -> f32) -> f32 {
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for sample in samples {
        let value = axis(*sample);
        min = min.min(value);
        max = max.max(value);
    }
    max - min
}

fn derivative_sign_inversions(samples: &[Vec3], axis: impl Fn(Vec3) -> f32) -> usize {
    let mut previous = 0.0_f32;
    let mut inversions = 0;
    for pair in samples.windows(2) {
        let delta = axis(pair[1]) - axis(pair[0]);
        if delta.abs() <= 0.00001 {
            continue;
        }
        let sign = delta.signum();
        if previous != 0.0 && sign != previous {
            inversions += 1;
        }
        previous = sign;
    }
    inversions
}

fn max_positive_y_delta(samples: &[Vec3]) -> f32 {
    samples
        .windows(2)
        .map(|pair| pair[1].y - pair[0].y)
        .fold(0.0, f32::max)
}

fn periodic_derivative_score(
    samples: &[Vec3],
    axis: impl Fn(Vec3) -> f32 + Copy,
    max_period: usize,
) -> f32 {
    let deltas: Vec<_> = samples
        .windows(2)
        .map(|pair| axis(pair[1]) - axis(pair[0]))
        .collect();
    (2..max_period.min(deltas.len() / 2))
        .map(|period| {
            deltas
                .iter()
                .zip(deltas.iter().skip(period))
                .map(|(a, b)| (a - b).abs())
                .sum::<f32>()
                / (deltas.len() - period) as f32
        })
        .fold(0.0, f32::max)
}

fn ascii_graph(samples: &[Vec3], axis: impl Fn(Vec3) -> f32) -> String {
    const WIDTH: usize = 80;
    const HEIGHT: usize = 12;
    let values: Vec<_> = (0..WIDTH)
        .map(|column| {
            let index = column * (samples.len() - 1) / (WIDTH - 1);
            axis(samples[index])
        })
        .collect();
    let min = values.iter().copied().fold(f32::INFINITY, f32::min);
    let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let span = (max - min).max(0.000001);
    let mut rows = vec![vec![' '; WIDTH]; HEIGHT];
    for (column, value) in values.iter().enumerate() {
        let normalized = (*value - min) / span;
        let row = HEIGHT - 1 - (normalized * (HEIGHT - 1) as f32).round() as usize;
        rows[row][column] = '*';
    }
    rows.into_iter()
        .map(|row| row.into_iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
