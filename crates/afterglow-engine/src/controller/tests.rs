use super::*;
use crate::input::{
    InputActionValue, InputAxis, InputAxisValue, PlayerCommand, PlayerCommandQueue,
};

fn command(axes: &[(&str, f32)], actions: &[InputActionValue]) -> PlayerCommand {
    PlayerCommand {
        player: NetworkPlayerId(1),
        axes: axes
            .iter()
            .map(|(axis, value)| InputAxisValue {
                axis: InputAxis::new(*axis),
                value: *value,
            })
            .collect(),
        actions: actions.to_vec(),
        ..default()
    }
}

#[test]
fn input_speed_scale_matches_amnesia_axis_multiplier() {
    assert_eq!(input_speed_scale(1.0, 0.0), 1.0);
    assert_eq!(input_speed_scale(0.0, 1.0), 1.0);
    assert!((input_speed_scale(1.0, 1.0) - 1.0).abs() < 0.0001);
    assert!((input_speed_scale(0.5, 0.5) - 0.5).abs() < 0.0001);
}

#[test]
fn digital_diagonal_movement_respects_side_speed_limit() {
    let config = FirstPersonControllerConfig {
        ground_accel: 1000.0,
        ..default()
    };
    let mut forward = FirstPersonMotorState {
        grounded: true,
        ..default()
    };
    let mut diagonal = forward;
    let forward_command = command(&[("move.y", 1.0)], &[]);
    let diagonal_command = command(&[("move.x", 1.0), ("move.y", 1.0)], &[]);

    integrate_first_person_motor(Some(&forward_command), &config, &mut forward, 1.0 / 60.0);
    integrate_first_person_motor(Some(&diagonal_command), &config, &mut diagonal, 1.0 / 60.0);

    let forward_speed = Vec2::new(forward.velocity.x, forward.velocity.z).length();
    let diagonal_speed = Vec2::new(diagonal.velocity.x, diagonal.velocity.z).length();

    assert!(diagonal_speed <= forward_speed);
    assert!(diagonal_speed > 0.0);
}

#[test]
fn one_frame_strafe_tap_uses_acceleration_not_max_speed_scaled_jump() {
    let config = FirstPersonControllerConfig::default();
    let mut state = FirstPersonMotorState {
        grounded: true,
        ..default()
    };
    let strafe = command(&[("move.x", 1.0)], &[]);
    let dt = 1.0 / 60.0;

    integrate_first_person_motor(Some(&strafe), &config, &mut state, dt);

    let horizontal_speed = Vec2::new(state.velocity.x, state.velocity.z).length();
    assert!((horizontal_speed - config.side_accel * dt).abs() < 0.001);
    assert!(horizontal_speed < 0.4);
}

#[test]
fn motor_moves_from_scripted_command_without_raw_input() {
    let config = FirstPersonControllerConfig {
        gravity: 0.0,
        ..default()
    };
    let mut state = FirstPersonMotorState {
        grounded: true,
        ..default()
    };
    let command = command(&[("move.y", 1.0)], &[]);

    integrate_first_person_motor(Some(&command), &config, &mut state, 1.0 / 60.0);

    assert!(state.velocity.z < 0.0);
    assert!(state.velocity.length() > 0.0);
}

#[test]
fn release_input_deaccelerates_local_speed_like_hpl2() {
    let config = FirstPersonControllerConfig {
        gravity: 0.0,
        ..default()
    };
    let mut state = FirstPersonMotorState {
        grounded: true,
        ..default()
    };
    let command = command(&[("move.y", 1.0)], &[]);

    for _ in 0..20 {
        integrate_first_person_motor(Some(&command), &config, &mut state, 1.0 / 60.0);
        state.grounded = true;
    }
    assert!(state.forward_speed > 0.0);

    for _ in 0..30 {
        integrate_first_person_motor(None, &config, &mut state, 1.0 / 60.0);
        state.grounded = true;
    }

    assert!(state.forward_speed.abs() < 0.2);
}

#[test]
fn airborne_move_uses_hpl2_in_air_speed_cap_for_jump_support() {
    let config = FirstPersonControllerConfig {
        gravity: 0.0,
        ground_accel: 1000.0,
        air_wish_speed: 0.2,
        ..default()
    };
    let mut grounded = FirstPersonMotorState {
        grounded: true,
        ..default()
    };
    let mut airborne = FirstPersonMotorState::default();
    let command = command(&[("move.y", 1.0)], &[]);

    integrate_first_person_motor(Some(&command), &config, &mut grounded, 1.0 / 60.0);
    integrate_first_person_motor(Some(&command), &config, &mut airborne, 1.0 / 60.0);

    assert!((grounded.forward_speed - config.ground_speed).abs() < 0.001);
    assert!((airborne.forward_speed - config.air_wish_speed).abs() < 0.001);
}

#[test]
fn jump_buffer_fires_while_grounded() {
    let config = FirstPersonControllerConfig::default();
    let mut state = FirstPersonMotorState {
        grounded: true,
        ..default()
    };
    let command = command(&[], &[InputActionValue::pressed("jump")]);

    integrate_first_person_motor(Some(&command), &config, &mut state, 1.0 / 60.0);

    assert!(!state.grounded);
    assert_eq!(state.velocity.y, config.jump_speed);
}

#[test]
fn jump_timer_relieves_gravity_like_amnesia_even_after_release() {
    let config = FirstPersonControllerConfig::default();
    let mut assisted = FirstPersonMotorState {
        velocity: Vec3::Y * config.jump_speed,
        jump_hold_ticks: config.jump_hold_ticks,
        ..default()
    };
    let mut unassisted = FirstPersonMotorState {
        velocity: Vec3::Y * config.jump_speed,
        ..default()
    };

    integrate_first_person_motor(None, &config, &mut assisted, 1.0 / 60.0);
    integrate_first_person_motor(None, &config, &mut unassisted, 1.0 / 60.0);

    assert!(assisted.velocity.y > unassisted.velocity.y);
}

#[test]
fn grounded_motor_keeps_gravity_for_hpl2_vertical_collision_pass() {
    let config = FirstPersonControllerConfig::default();
    let mut state = FirstPersonMotorState {
        grounded: true,
        ..default()
    };

    integrate_first_person_motor(None, &config, &mut state, 1.0 / 60.0);

    assert!((state.velocity.y + config.gravity / 60.0).abs() < 0.001);
}

#[test]
fn coyote_jump_works_after_leaving_ground() {
    let config = FirstPersonControllerConfig::default();
    let mut state = FirstPersonMotorState {
        grounded: true,
        ..default()
    };

    integrate_first_person_motor(None, &config, &mut state, 1.0 / 60.0);
    state.grounded = false;
    let command = command(&[], &[InputActionValue::pressed("jump")]);
    integrate_first_person_motor(Some(&command), &config, &mut state, 1.0 / 60.0);

    assert_eq!(state.velocity.y, config.jump_speed);
}

#[test]
fn ground_contact_hysteresis_survives_short_contact_gaps() {
    let mut state = FirstPersonMotorState::default();

    update_ground_contact(true, Vec3::Y, 3, &mut state);
    update_ground_contact(false, Vec3::Y, 3, &mut state);
    update_ground_contact(false, Vec3::Y, 3, &mut state);

    assert!(state.grounded);

    update_ground_contact(false, Vec3::Y, 3, &mut state);

    assert!(!state.grounded);
    assert_eq!(state.ground_normal, Vec3::Y);
}

#[test]
fn movement_projects_onto_ground_normal() {
    let wish = Vec3::Z;
    let slope_normal = Vec3::new(0.0, 1.0, -1.0).normalize();
    let projected = project_move_on_ground(wish, slope_normal);

    assert!((projected.length() - 1.0).abs() < 0.001);
    assert!(projected.dot(slope_normal).abs() < 0.001);
}

#[test]
fn grounded_slope_movement_preserves_aligned_vertical_velocity() {
    let config = FirstPersonControllerConfig {
        gravity: 0.0,
        ground_accel: 1000.0,
        ..default()
    };
    let mut state = FirstPersonMotorState {
        grounded: true,
        ground_normal: Vec3::new(0.0, 1.0, 0.5).normalize(),
        ..default()
    };
    let command = command(&[("move.y", 1.0)], &[]);

    integrate_first_person_motor(Some(&command), &config, &mut state, 1.0 / 60.0);

    assert_eq!(state.velocity.y, 0.0);
    let delta = local_move_delta_from_speeds(&state, 1.0 / 60.0);
    assert!(delta.y > 0.0);
    assert!(delta.dot(state.ground_normal).abs() < 0.001);
}

#[test]
fn grounded_hpl2_gravity_does_not_create_downhill_slide() {
    let config = FirstPersonControllerConfig::default();
    let mut state = FirstPersonMotorState {
        grounded: true,
        ground_normal: Vec3::new(0.0, 1.0, 0.5).normalize(),
        ..default()
    };
    integrate_first_person_motor(None, &config, &mut state, 1.0 / 60.0);
    assert_eq!(state.velocity.x, 0.0);
    assert_eq!(state.velocity.z, 0.0);
    assert!(state.velocity.y < 0.0);
}

#[test]
fn crouch_action_lowers_target_speed() {
    let config = FirstPersonControllerConfig {
        ground_accel: 1000.0,
        ..default()
    };
    let mut standing = FirstPersonMotorState {
        grounded: true,
        ..default()
    };
    let mut crouching = standing;
    let move_command = command(&[("move.y", 1.0)], &[]);
    let crouch_command = command(
        &[("move.y", 1.0)],
        &[InputActionValue::held(config.crouch_action.clone())],
    );

    for _ in 0..20 {
        integrate_first_person_motor(Some(&move_command), &config, &mut standing, 1.0 / 60.0);
        standing.grounded = true;
        integrate_first_person_motor(Some(&crouch_command), &config, &mut crouching, 1.0 / 60.0);
        crouching.grounded = true;
    }

    assert_eq!(crouching.desired_stance, ControllerStance::Crouching);
    let standing_speed = Vec2::new(standing.velocity.x, standing.velocity.z).length();
    let crouching_speed = Vec2::new(crouching.velocity.x, crouching.velocity.z).length();
    assert!(crouching_speed < standing_speed);
}

#[test]
fn default_crouch_is_hold_not_toggle() {
    let config = FirstPersonControllerConfig::default();
    let mut state = FirstPersonMotorState {
        grounded: true,
        ..default()
    };
    let crouch = command(&[], &[InputActionValue::held(config.crouch_action.clone())]);

    integrate_first_person_motor(Some(&crouch), &config, &mut state, 1.0 / 60.0);
    assert_eq!(state.desired_stance, ControllerStance::Crouching);

    integrate_first_person_motor(None, &config, &mut state, 1.0 / 60.0);
    assert_eq!(state.desired_stance, ControllerStance::Standing);
}

#[test]
fn toggle_crouch_can_be_enabled_for_games_that_want_it() {
    let config = FirstPersonControllerConfig {
        toggle_crouch: true,
        ..default()
    };
    let mut state = FirstPersonMotorState {
        grounded: true,
        ..default()
    };
    let crouch_press = command(
        &[],
        &[InputActionValue::pressed(config.crouch_action.clone())],
    );

    integrate_first_person_motor(Some(&crouch_press), &config, &mut state, 1.0 / 60.0);
    assert_eq!(state.desired_stance, ControllerStance::Crouching);

    integrate_first_person_motor(None, &config, &mut state, 1.0 / 60.0);
    assert_eq!(state.desired_stance, ControllerStance::Crouching);

    integrate_first_person_motor(Some(&crouch_press), &config, &mut state, 1.0 / 60.0);
    assert_eq!(state.desired_stance, ControllerStance::Standing);
}

#[test]
fn sprinting_while_moving_auto_requests_stand_like_amnesia() {
    let config = FirstPersonControllerConfig::default();
    let mut state = FirstPersonMotorState {
        grounded: true,
        stance: ControllerStance::Crouching,
        desired_stance: ControllerStance::Crouching,
        ..default()
    };
    let command = command(
        &[("move.y", 1.0)],
        &[InputActionValue::held(config.sprint_action.clone())],
    );

    integrate_first_person_motor(Some(&command), &config, &mut state, 1.0 / 60.0);

    assert_eq!(state.desired_stance, ControllerStance::Standing);
}

#[test]
fn stance_center_delta_keeps_feet_stable() {
    let config = FirstPersonControllerConfig::default();

    assert!(
        (feet_stable_center_delta(
            &config,
            ControllerStance::Standing,
            ControllerStance::Crouching
        ) + 0.325)
            .abs()
            < 0.001
    );
    assert!(
        (feet_stable_center_delta(
            &config,
            ControllerStance::Crouching,
            ControllerStance::Standing
        ) - 0.325)
            .abs()
            < 0.001
    );
}

#[test]
fn step_height_uses_configured_min_and_max() {
    let config = FirstPersonControllerConfig::default();

    assert!(!is_step_height_allowed(
        config.min_step_height * 0.5,
        &config
    ));
    assert!(is_step_height_allowed(
        (config.min_step_height + config.max_step_height) * 0.5,
        &config
    ));
    assert!(!is_step_height_allowed(
        config.max_step_height * 1.5,
        &config
    ));
}

#[test]
fn command_queue_routes_by_player_id() {
    let mut queue = PlayerCommandQueue::default();
    queue.replace(vec![
        PlayerCommand {
            player: NetworkPlayerId(1),
            tick: 7,
            ..default()
        },
        PlayerCommand {
            player: NetworkPlayerId(2),
            tick: 9,
            ..default()
        },
    ]);
    let lookup = commands::PlayerCommandLookup::new(&queue);

    assert_eq!(
        lookup.get(NetworkPlayerId(2)).map(|command| command.tick),
        Some(9)
    );
}
