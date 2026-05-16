use super::*;
use crate::network::NetworkPlayerId;

fn controller() -> FirstPersonController {
    FirstPersonController::new(NetworkPlayerId(1))
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

fn bob_from_state(state: &FirstPersonCameraState) -> Vec3 {
    hpl2_head_bob(
        state.bobbing,
        state.bob_phase,
        state.current_bob_amplitude,
        state.landing_bounce,
    )
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

#[test]
fn default_camera_values_match_hpl2_bob_with_faster_crouch() {
    let config = FirstPersonCameraConfig::default();
    assert_eq!(config.walk_bob_amplitude, Vec2::new(0.03, 0.03));
    assert_eq!(config.run_bob_amplitude, Vec2::new(0.05, 0.06));
    assert_eq!(config.crouch_bob_amplitude, Vec2::new(0.06, 0.04));
    assert_eq!(config.walk_bob_min_speed, 0.4);
    assert_eq!(config.walk_bob_max_speed, 1.8);
    assert_eq!(config.run_bob_min_speed, 0.5);
    assert_eq!(config.run_bob_max_speed, 2.5);
    assert_eq!(config.crouch_bob_min_speed, 0.2);
    assert_eq!(config.crouch_bob_max_speed, 1.2);
    assert_eq!(config.bob_blend_speed, 0.1);
    assert_eq!(config.crouch_down_head_speed, 3.0);
    assert_eq!(config.stand_up_head_speed, 3.6);
    assert_eq!(config.crouch_head_slow_distance, 0.05);
    assert_eq!(config.ground_bounce_size, 0.08);
    assert_eq!(config.ground_bounce_speed, 2.8);
    assert_eq!(config.min_hit_ground_bounce_speed, 5.0);
}

#[test]
fn head_bob_uses_amnesia_formula() {
    let config = FirstPersonCameraConfig::default();
    let state = FirstPersonCameraState {
        bob_phase: std::f32::consts::FRAC_PI_2,
        bobbing: true,
        current_bob_amplitude: config.walk_bob_amplitude,
        ..default()
    };
    let bob = bob_from_state(&state);

    assert!((bob.x - 0.0).abs() < 0.0001);
    assert!((bob.y - 0.0).abs() < 0.0001);
}

#[test]
fn side_strafe_drives_head_bob_like_amnesia() {
    let config = FirstPersonCameraConfig::default();
    let mut state = FirstPersonCameraState::default();
    let motor = FirstPersonMotorState {
        grounded: true,
        side_speed: 2.0,
        velocity: Vec3::X * 2.0,
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
}

#[test]
fn side_strafe_does_not_auto_tilt_camera() {
    let config = FirstPersonCameraConfig::default();
    let mut state = FirstPersonCameraState::default();
    let motor = FirstPersonMotorState {
        grounded: true,
        side_speed: 4.0,
        velocity: Vec3::X * 4.0,
        ..default()
    };

    update_camera_state(
        &config,
        &mut state,
        &target(),
        &controller(),
        &motor,
        Vec3::ZERO,
        0.5,
    );

    assert_eq!(state.roll, 0.0);
}

#[test]
fn bob_speed_interpolates_like_amnesia() {
    let config = FirstPersonCameraConfig::default();
    let controller = controller();
    let motor = FirstPersonMotorState {
        grounded: true,
        ..default()
    };
    let speed = Vec2::new(controller.config.ground_speed * 0.5, 0.0);

    assert!((bob_speed(&config, &motor, &controller, speed) - 1.1).abs() < 0.0001);
}

#[test]
fn camera_adds_landing_bounce_on_ground_transition() {
    let config = FirstPersonCameraConfig::default();
    let mut hard_landing = FirstPersonCameraState {
        was_grounded: false,
        last_vertical_velocity: -config.min_hit_ground_bounce_speed * 2.5,
        ..default()
    };
    let mut medium_landing = FirstPersonCameraState {
        was_grounded: false,
        last_vertical_velocity: -config.min_hit_ground_bounce_speed * 1.5,
        ..default()
    };
    let mut soft_landing = FirstPersonCameraState {
        was_grounded: false,
        last_vertical_velocity: -2.0,
        ..default()
    };
    let motor = FirstPersonMotorState {
        grounded: true,
        ..default()
    };

    for landing in [&mut hard_landing, &mut medium_landing, &mut soft_landing] {
        update_camera_state(
            &config,
            landing,
            &target(),
            &controller(),
            &motor,
            Vec3::ZERO,
            0.016,
        );
    }

    assert!(hard_landing.landing_bounce < medium_landing.landing_bounce);
    assert!(medium_landing.landing_bounce < 0.0);
    assert!(hard_landing.landing_bounce_phase < 1.0);
    assert!(medium_landing.landing_bounce_phase < 1.0);
    assert_eq!(soft_landing.landing_bounce, 0.0);
    assert_eq!(soft_landing.landing_bounce_phase, 1.0);
}

#[test]
fn camera_impulses_decay_toward_zero() {
    let mut state = FirstPersonCameraState {
        impulse_pitch: 1.0,
        impulse_yaw: -1.0,
        impulse_roll: 0.5,
        ..default()
    };

    decay_impulses(&mut state, 10.0, 0.1);

    assert!(state.impulse_pitch.abs() < 1.0);
    assert!(state.impulse_yaw.abs() < 1.0);
    assert!(state.impulse_roll.abs() < 0.5);
}

#[test]
fn camera_fov_smooths_toward_sprint_goal() {
    let config = FirstPersonCameraConfig::default();
    let mut state = FirstPersonCameraState {
        fov: config.fov,
        ..default()
    };
    let mut motor = FirstPersonMotorState {
        grounded: true,
        forward_speed: 8.0,
        velocity: Vec3::NEG_Z * 8.0,
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
    assert!(state.fov > config.fov);
    assert!(state.fov < config.fov + config.sprint_fov_add);

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
    assert!(state.fov > config.fov);
}

#[test]
fn camera_footstep_fires_once_per_bob_half_cycle() {
    let config = FirstPersonCameraConfig::default();
    let mut state = FirstPersonCameraState {
        current_bob_amplitude: config.walk_bob_amplitude,
        ..default()
    };
    let motor = FirstPersonMotorState {
        grounded: true,
        forward_speed: 2.0,
        velocity: Vec3::NEG_Z * 2.0,
        ..default()
    };

    let first = update_camera_state(
        &config,
        &mut state,
        &target(),
        &controller(),
        &motor,
        Vec3::ZERO,
        0.1,
    );
    let before = state.footstep_count;
    let second = update_camera_state(
        &config,
        &mut state,
        &target(),
        &controller(),
        &motor,
        Vec3::ZERO,
        0.6,
    );

    assert!(!first);
    assert!(second);
    assert_eq!(state.footstep_count, before + 1);
}

#[test]
fn camera_bob_offset_is_rotated_once_by_final_camera_rotation() {
    let config = FirstPersonCameraConfig {
        walk_bob_amplitude: Vec2::new(0.03, 0.03),
        ..default()
    };
    let state = FirstPersonCameraState {
        smoothed_position: Vec3::new(1.0, 2.0, 3.0),
        bob_phase: std::f32::consts::FRAC_PI_2,
        bobbing: true,
        current_bob_amplitude: config.walk_bob_amplitude,
        ..default()
    };
    let motor = FirstPersonMotorState {
        yaw: std::f32::consts::FRAC_PI_2,
        grounded: true,
        forward_speed: 1.0,
        ..default()
    };
    let mut transform = Transform::default();

    apply_camera_transform(&state, &motor, &mut transform);

    let expected = state.smoothed_position + transform.rotation * bob_from_state(&state);
    assert!(transform.translation.abs_diff_eq(expected, 0.001));
}
