use super::*;

fn controller() -> FirstPersonController {
    FirstPersonController::new()
}

#[test]
fn camera_eye_height_smooths_toward_crouch_height() {
    let config = FirstPersonCameraConfig::default();
    let mut state = FirstPersonCameraState {
        eye_height: config.standing_eye_height,
        ..default()
    };
    let motor = FirstPersonMotorState {
        stance: ControllerStance::Crouching,
        ..default()
    };

    update_camera_state(
        &config,
        &mut state,
        &Transform::from_xyz(0.0, 1.0, 0.0),
        &controller(),
        &motor,
        Vec3::ZERO,
        0.1,
    );

    assert!((state.eye_height - 1.28).abs() < 0.0001);
}

#[test]
fn crouch_camera_height_is_feet_relative_not_center_relative() {
    let config = FirstPersonCameraConfig::default();
    let controller = controller();
    let mut state = FirstPersonCameraState {
        initialized: true,
        eye_height: config.standing_eye_height,
        was_grounded: true,
        ..default()
    };
    let motor = FirstPersonMotorState {
        stance: ControllerStance::Crouching,
        grounded: true,
        ..default()
    };
    let crouched_center_y = controller.config.crouching_height * 0.5;

    update_camera_state(
        &config,
        &mut state,
        &Transform::from_xyz(0.0, crouched_center_y, 0.0),
        &controller,
        &motor,
        Vec3::ZERO,
        1.0,
    );

    assert!((state.smoothed_position.y - config.crouching_eye_height).abs() < 0.001);
}
