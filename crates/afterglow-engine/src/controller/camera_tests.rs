use super::*;

fn controller() -> FirstPersonController {
    FirstPersonController::new()
}

fn target() -> Transform {
    Transform::from_xyz(0.0, 1.0, 0.0)
}

fn center_for_camera_y(
    controller: &FirstPersonController,
    stance: ControllerStance,
    camera_y: f32,
) -> f32 {
    camera_y + controller.config.height(stance) * 0.5
        - FirstPersonCameraConfig::default().standing_eye_height
}

#[test]
fn camera_initializes_position_once_even_at_world_origin() {
    let config = FirstPersonCameraConfig::default();
    let mut state = FirstPersonCameraState::default();
    let motor = FirstPersonMotorState::default();

    update_camera_state(
        &config,
        &mut state,
        &Transform::from_xyz(
            0.0,
            center_for_camera_y(&controller(), ControllerStance::Standing, 0.0),
            0.0,
        ),
        &controller(),
        &motor,
        Vec3::ZERO,
        0.1,
    );
    assert!(state.initialized);
    assert_eq!(state.smoothed_position, Vec3::ZERO);

    update_camera_state(
        &config,
        &mut state,
        &Transform::from_xyz(2.0, -config.standing_eye_height, 3.0),
        &controller(),
        &motor,
        Vec3::ZERO,
        0.1,
    );

    assert_eq!(state.smoothed_position.x, 2.0);
    assert_eq!(state.smoothed_position.z, 3.0);
}

#[test]
fn camera_tracks_horizontal_body_motion_without_smoothing_lag() {
    let config = FirstPersonCameraConfig {
        position_smooth_speed: 1.0,
        ..default()
    };
    let mut state = FirstPersonCameraState {
        initialized: true,
        smoothed_position: Vec3::ZERO,
        was_grounded: true,
        ..default()
    };
    let motor = FirstPersonMotorState::default();

    update_camera_state(
        &config,
        &mut state,
        &Transform::from_xyz(10.0, 0.0, -5.0),
        &controller(),
        &motor,
        Vec3::ZERO,
        0.016,
    );

    assert_eq!(state.smoothed_position.x, 10.0);
    assert_eq!(state.smoothed_position.z, -5.0);
    assert!(state.smoothed_position.y < config.standing_eye_height);
}

#[test]
fn camera_tracks_grounded_vertical_body_motion_without_slope_lag() {
    let config = FirstPersonCameraConfig {
        position_smooth_speed: 1.0,
        ..default()
    };
    let mut state = FirstPersonCameraState {
        initialized: true,
        smoothed_position: Vec3::ZERO,
        was_grounded: true,
        ..default()
    };
    let motor = FirstPersonMotorState {
        grounded: true,
        ..default()
    };

    update_camera_state(
        &config,
        &mut state,
        &Transform::from_xyz(0.0, 2.0, 0.0),
        &controller(),
        &motor,
        Vec3::ZERO,
        0.016,
    );

    let expected = 2.0 + config.standing_eye_height - controller().config.standing_height * 0.5;
    assert!((state.smoothed_position.y - expected).abs() < 0.0001);
}

#[test]
fn camera_smooths_vertical_body_motion_while_step_climbing() {
    let config = FirstPersonCameraConfig {
        position_smooth_speed: 1.0,
        ..default()
    };
    let mut state = FirstPersonCameraState {
        initialized: true,
        smoothed_position: Vec3::new(1.0, 0.5, -1.0),
        was_grounded: true,
        ..default()
    };
    let motor = FirstPersonMotorState {
        grounded: true,
        climbing: true,
        ..default()
    };
    let target_y = 1.2;

    update_camera_state(
        &config,
        &mut state,
        &Transform::from_xyz(
            10.0,
            center_for_camera_y(&controller(), ControllerStance::Standing, target_y),
            -5.0,
        ),
        &controller(),
        &motor,
        Vec3::ZERO,
        0.016,
    );

    assert_eq!(state.smoothed_position.x, 10.0);
    assert_eq!(state.smoothed_position.z, -5.0);
    assert!(state.smoothed_position.y > 0.5);
    assert!(state.smoothed_position.y < target_y);
    assert!(state.was_climbing);
}

#[test]
fn camera_keeps_smoothing_after_step_climb_until_vertical_error_is_small() {
    let config = FirstPersonCameraConfig {
        position_smooth_speed: 1.0,
        ..default()
    };
    let target_y = 1.2;
    let target_transform = Transform::from_xyz(
        0.0,
        center_for_camera_y(&controller(), ControllerStance::Standing, target_y),
        0.0,
    );
    let motor = FirstPersonMotorState {
        grounded: true,
        climbing: false,
        ..default()
    };
    let mut state = FirstPersonCameraState {
        initialized: true,
        smoothed_position: Vec3::new(0.0, 1.0, 0.0),
        was_grounded: true,
        was_climbing: true,
        ..default()
    };

    update_camera_state(
        &config,
        &mut state,
        &target_transform,
        &controller(),
        &motor,
        Vec3::ZERO,
        0.016,
    );
    assert!(state.smoothed_position.y > 1.0);
    assert!(state.smoothed_position.y < target_y);
    assert!(state.was_climbing);

    state.smoothed_position.y = target_y - 0.001;
    update_camera_state(
        &config,
        &mut state,
        &target_transform,
        &controller(),
        &motor,
        Vec3::ZERO,
        0.016,
    );
    assert!(!state.was_climbing);
}

#[test]
fn camera_smooths_vertical_position_on_landing() {
    let config = FirstPersonCameraConfig {
        position_smooth_speed: 1.0,
        ..default()
    };
    let mut state = FirstPersonCameraState {
        initialized: true,
        smoothed_position: Vec3::new(0.0, 4.0, 0.0),
        was_grounded: false,
        last_vertical_velocity: -8.0,
        ..default()
    };
    let motor = FirstPersonMotorState {
        grounded: true,
        ..default()
    };

    update_camera_state(
        &config,
        &mut state,
        &Transform::from_xyz(0.0, 0.0, 0.0),
        &controller(),
        &motor,
        Vec3::ZERO,
        0.016,
    );

    assert!(state.smoothed_position.y > config.standing_eye_height);
    assert!(state.smoothed_position.y < 4.0);
}

#[test]
fn camera_bob_blends_in_only_when_grounded_and_moving() {
    let config = FirstPersonCameraConfig::default();
    let mut state = FirstPersonCameraState::default();
    let mut motor = FirstPersonMotorState {
        grounded: true,
        forward_speed: 2.0,
        velocity: Vec3::NEG_Z * 2.0,
        ..default()
    };

    update_camera_state(
        &config,
        &mut state,
        &target(),
        &controller(),
        &motor,
        Vec3::ZERO,
        0.1,
    );
    assert!(state.bobbing);
    assert!(state.current_bob_amplitude.length() > 0.0);

    motor.velocity = Vec3::ZERO;
    update_camera_state(
        &config,
        &mut state,
        &target(),
        &controller(),
        &motor,
        Vec3::ZERO,
        0.1,
    );
    assert!(state.current_bob_amplitude.length() < config.walk_bob_amplitude.length());
}

#[test]
fn camera_uses_actual_velocity_not_blocked_movement_intent() {
    let config = FirstPersonCameraConfig::default();
    let mut state = FirstPersonCameraState {
        fov: config.fov,
        ..default()
    };
    let motor = FirstPersonMotorState {
        grounded: true,
        forward_speed: 5.0,
        velocity: Vec3::ZERO,
        ..default()
    };

    update_camera_state(
        &config,
        &mut state,
        &target(),
        &controller(),
        &motor,
        Vec3::ZERO,
        0.1,
    );

    assert!(!state.bobbing);
    assert_eq!(state.current_bob_amplitude, Vec2::ZERO);
    assert_eq!(state.fov, config.fov);
}
