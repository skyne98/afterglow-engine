use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;

use crate::input::AfterglowAction;

use super::{
    util, ControllerStance, FirstPersonControllerConfig, FirstPersonMotorState,
    is_walkable_normal,
};

pub fn integrate_first_person_motor(
    action_state: Option<&ActionState<AfterglowAction>>,
    config: &FirstPersonControllerConfig,
    state: &mut FirstPersonMotorState,
    dt: f32,
) {
    integrate_first_person_look(action_state, config, state);
    let input = integrate_first_person_input(action_state, config, state, dt);
    write_horizontal_velocity_from_local_speeds(state);
    if !input.jumped {
        apply_first_person_gravity(config, state, dt);
    }
}

pub fn integrate_first_person_look(
    action_state: Option<&ActionState<AfterglowAction>>,
    config: &FirstPersonControllerConfig,
    state: &mut FirstPersonMotorState,
) {
    let command = FirstPersonCommandState::from_action_state(action_state);
    integrate_first_person_command_look(&command, config, state);
}

pub fn integrate_first_person_command_look(
    command: &FirstPersonCommandState,
    config: &FirstPersonControllerConfig,
    state: &mut FirstPersonMotorState,
) {
    state.yaw -= command.look_axis.x * config.look_sensitivity.x;
    state.pitch = (state.pitch - command.look_axis.y * config.look_sensitivity.y)
        .clamp(config.min_pitch, config.max_pitch);
}

pub fn clamp_local_speeds_to_actual_stance(
    command: &FirstPersonCommandState,
    config: &FirstPersonControllerConfig,
    state: &mut FirstPersonMotorState,
) {
    if !state.grounded {
        return;
    }
    let limits = local_speed_limits(command, config, state.stance, state.grounded);
    state.forward_speed = state.forward_speed.clamp(-limits.backward, limits.forward);
    state.side_speed = state.side_speed.clamp(-limits.side, limits.side);
}

#[derive(Clone, Debug)]
pub struct FirstPersonInputStep {
    pub command: FirstPersonCommandState,
    pub jumped: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FirstPersonCommandState {
    pub tick: u32,
    pub move_axis: Vec2,
    pub look_axis: Vec2,
    pub jump_pressed: bool,
    pub jump_held: bool,
    pub crouch_pressed: bool,
    pub crouch_held: bool,
    pub sprint_pressed: bool,
    pub sprint_held: bool,
}

impl FirstPersonCommandState {
    fn from_action_state(action_state: Option<&ActionState<AfterglowAction>>) -> Self {
        let Some(action_state) = action_state else {
            return Self::default();
        };
        Self {
            tick: 0,
            move_axis: action_state.clamped_axis_pair(&AfterglowAction::Move),
            look_axis: action_state.axis_pair(&AfterglowAction::Look),
            jump_pressed: action_state.just_pressed(&AfterglowAction::Jump),
            jump_held: action_state.pressed(&AfterglowAction::Jump),
            crouch_pressed: action_state.just_pressed(&AfterglowAction::Crouch),
            crouch_held: action_state.pressed(&AfterglowAction::Crouch),
            sprint_pressed: action_state.just_pressed(&AfterglowAction::Sprint),
            sprint_held: action_state.pressed(&AfterglowAction::Sprint),
        }
    }

    pub fn jump_down(self) -> bool {
        self.jump_pressed || self.jump_held
    }

    pub fn crouch_down(self) -> bool {
        self.crouch_pressed || self.crouch_held
    }

    pub fn sprint_down(self) -> bool {
        self.sprint_pressed || self.sprint_held
    }
}

pub fn integrate_first_person_input(
    action_state: Option<&ActionState<AfterglowAction>>,
    config: &FirstPersonControllerConfig,
    state: &mut FirstPersonMotorState,
    dt: f32,
) -> FirstPersonInputStep {
    let command = FirstPersonCommandState::from_action_state(action_state);
    integrate_first_person_command(&command, config, state, dt)
}

pub fn integrate_first_person_command(
    command: &FirstPersonCommandState,
    config: &FirstPersonControllerConfig,
    state: &mut FirstPersonMotorState,
    dt: f32,
) -> FirstPersonInputStep {
    if config.toggle_crouch {
        if command.crouch_pressed {
            state.desired_stance = if state.desired_stance == ControllerStance::Crouching {
                ControllerStance::Standing
            } else {
                ControllerStance::Crouching
            };
        }
    } else {
        state.desired_stance = if command.crouch_down() {
            ControllerStance::Crouching
        } else {
            ControllerStance::Standing
        };
    }

    let move_x = command.move_axis.x;
    let move_y = command.move_axis.y;
    if state.grounded
        && state.desired_stance == ControllerStance::Crouching
        && (move_x != 0.0 || move_y != 0.0)
        && command.sprint_down()
    {
        state.desired_stance = ControllerStance::Standing;
    }
    update_local_move_speeds(&command, config, state, move_x, move_y, dt);
    let jump_down = command.jump_down();
    let jump_requested = update_jump_input_latch(jump_down, config, state);
    update_jump_windows(jump_requested, config, state);

    let can_jump = config.jump_enabled
        && (has_jump_floor(config, state) || (!state.grounded && state.coyote_ticks > 0));
    let jumped = can_jump && state.jump_buffer_ticks > 0;
    if jumped {
        state.desired_stance = ControllerStance::Standing;
        state.velocity.y = config.jump_speed;
        state.grounded = false;
        state.ground_contact_ticks = 0;
        state.coyote_ticks = 0;
        state.jump_buffer_ticks = 0;
        state.jump_hold_ticks = config.jump_hold_ticks;
    }
    FirstPersonInputStep {
        command: *command,
        jumped,
    }
}

fn update_local_move_speeds(
    command: &FirstPersonCommandState,
    config: &FirstPersonControllerConfig,
    state: &mut FirstPersonMotorState,
    move_x: f32,
    move_y: f32,
    dt: f32,
) {
    let forward_input = move_y.clamp(-1.0, 1.0);
    let side_input = move_x.clamp(-1.0, 1.0);
    let diagonal_mul = if forward_input != 0.0 && side_input != 0.0 {
        std::f32::consts::FRAC_1_SQRT_2
    } else {
        1.0
    };
    let limits = local_speed_limits(command, config, state.desired_stance, state.grounded);
    let can_deaccel = state.grounded || config.deaccelerate_in_air;
    let forward_reverse_mul = if state.grounded {
        config.opposite_dir_accel_mul
    } else {
        1.0
    };
    let side_reverse_mul = if state.grounded {
        config.side_opposite_dir_accel_mul
    } else {
        1.0
    };
    state.forward_speed = update_axis_speed(AxisMove {
        speed: state.forward_speed,
        input: forward_input,
        max_positive: limits.forward * diagonal_mul,
        max_negative: -limits.backward * diagonal_mul,
        accel: config.ground_accel,
        deaccel: config.ground_deaccel,
        opposite_dir_accel_mul: forward_reverse_mul,
        can_deaccel,
        dt,
    });
    state.side_speed = update_axis_speed(AxisMove {
        speed: state.side_speed,
        input: side_input,
        max_positive: limits.side * diagonal_mul,
        max_negative: -limits.side * diagonal_mul,
        accel: config.side_accel,
        deaccel: config.side_deaccel,
        opposite_dir_accel_mul: side_reverse_mul,
        can_deaccel,
        dt,
    });
}

#[derive(Clone, Copy)]
struct LocalSpeedLimits {
    forward: f32,
    backward: f32,
    side: f32,
}

fn local_speed_limits(
    command: &FirstPersonCommandState,
    config: &FirstPersonControllerConfig,
    stance: ControllerStance,
    grounded: bool,
) -> LocalSpeedLimits {
    if !grounded {
        return LocalSpeedLimits {
            forward: config.air_wish_speed,
            backward: config.air_wish_speed,
            side: config.air_wish_speed,
        };
    }
    let forward = target_forward_speed(command, config, stance);
    let ground_ratio = config.ground_speed.max(f32::EPSILON);
    let stance_ratio = forward / ground_ratio;
    LocalSpeedLimits {
        forward,
        backward: config.backward_speed * stance_ratio,
        side: config.side_speed * stance_ratio,
    }
}

struct AxisMove {
    speed: f32,
    input: f32,
    max_positive: f32,
    max_negative: f32,
    accel: f32,
    deaccel: f32,
    opposite_dir_accel_mul: f32,
    can_deaccel: bool,
    dt: f32,
}

fn update_axis_speed(axis: AxisMove) -> f32 {
    let AxisMove {
        mut speed,
        input,
        max_positive,
        max_negative,
        accel,
        deaccel,
        opposite_dir_accel_mul,
        can_deaccel,
        dt,
    } = axis;
    if input == 0.0 {
        return if can_deaccel {
            move_toward_zero(speed, deaccel * dt)
        } else {
            speed
        };
    }
    if can_deaccel {
        if speed > max_positive {
            speed = (speed - deaccel * dt).max(max_positive);
        } else if speed < max_negative {
            speed = (speed + deaccel * dt).min(max_negative);
        }
    }
    let reverse = (input > 0.0 && speed < 0.0) || (input < 0.0 && speed > 0.0);
    let accel_mul = if reverse { opposite_dir_accel_mul } else { 1.0 };
    let speed_add = input * accel * accel_mul * dt;
    if speed_add > 0.0 && speed < max_positive {
        (speed + speed_add).min(max_positive)
    } else if speed_add < 0.0 && speed > max_negative {
        (speed + speed_add).max(max_negative)
    } else {
        speed
    }
}

fn move_toward_zero(speed: f32, amount: f32) -> f32 {
    if speed > 0.0 {
        (speed - amount).max(0.0)
    } else if speed < 0.0 {
        (speed + amount).min(0.0)
    } else {
        0.0
    }
}

fn update_jump_windows(
    jump_pressed: bool,
    config: &FirstPersonControllerConfig,
    state: &mut FirstPersonMotorState,
) {
    state.coyote_ticks = if has_jump_floor(config, state) {
        config.coyote_ticks
    } else if state.grounded {
        0
    } else {
        state.coyote_ticks.saturating_sub(1)
    };
    state.jump_buffer_ticks = if jump_pressed {
        config.jump_buffer_ticks
    } else {
        state.jump_buffer_ticks.saturating_sub(1)
    };
}

fn target_forward_speed(
    command: &FirstPersonCommandState,
    config: &FirstPersonControllerConfig,
    stance: ControllerStance,
) -> f32 {
    if stance == ControllerStance::Crouching {
        config.crouch_speed
    } else if command.sprint_down() {
        config.sprint_speed
    } else {
        config.ground_speed
    }
}

fn write_horizontal_velocity_from_local_speeds(state: &mut FirstPersonMotorState) {
    let (forward, right) = util::local_basis(state.yaw);
    let horizontal = forward * state.forward_speed + right * state.side_speed;
    state.velocity.x = horizontal.x;
    state.velocity.z = horizontal.z;
}

pub fn local_move_delta_from_speeds(state: &FirstPersonMotorState, dt: f32) -> Vec3 {
    let (forward, right) = util::local_basis(state.yaw);
    let delta = (forward * state.forward_speed + right * state.side_speed) * dt;
    if state.grounded && state.velocity.y <= 0.0 && delta.length_squared() > 0.0 {
        project_move_on_ground(delta.normalize(), state.ground_normal) * delta.length()
    } else {
        delta
    }
}

pub fn write_flat_horizontal_velocity_from_delta(
    state: &mut FirstPersonMotorState,
    delta: Vec3,
    dt: f32,
) {
    if dt <= f32::EPSILON {
        state.velocity.x = 0.0;
        state.velocity.z = 0.0;
        return;
    }
    state.velocity.x = delta.x / dt;
    state.velocity.z = delta.z / dt;
}

pub fn sync_local_speeds_from_velocity(state: &mut FirstPersonMotorState) {
    let local_speed = local_speeds_from_velocity(state);
    state.forward_speed = local_speed.x;
    state.side_speed = local_speed.y;
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct ReplayCommand(pub FirstPersonCommandState);

pub fn local_speeds_from_velocity(state: &FirstPersonMotorState) -> Vec2 {
    let (forward, right) = util::local_basis(state.yaw);
    let horizontal = Vec3::new(state.velocity.x, 0.0, state.velocity.z);
    Vec2::new(horizontal.dot(forward), horizontal.dot(right))
}

pub fn input_speed_scale(x: f32, y: f32) -> f32 {
    scaled_move_input(x, y).length().min(1.0)
}

fn scaled_move_input(x: f32, y: f32) -> Vec2 {
    let mut input = Vec2::new(x.clamp(-1.0, 1.0), y.clamp(-1.0, 1.0));
    if input.x != 0.0 && input.y != 0.0 {
        input *= std::f32::consts::FRAC_1_SQRT_2;
    }
    input
}

pub fn project_move_on_ground(wish_dir: Vec3, ground_normal: Vec3) -> Vec3 {
    let normal = ground_normal.normalize_or_zero();
    if normal == Vec3::ZERO || normal == Vec3::Y {
        return wish_dir;
    }
    let right = wish_dir.cross(normal).normalize_or_zero();
    if right == Vec3::ZERO {
        return wish_dir;
    }
    normal.cross(right).normalize_or_zero()
}

pub fn apply_first_person_gravity(
    config: &FirstPersonControllerConfig,
    state: &mut FirstPersonMotorState,
    dt: f32,
) {
    if state.climbing {
        state.jump_hold_ticks = 0;
        return;
    }
    let mut gravity = config.gravity;
    if config.jump_enabled && state.velocity.y > 0.0 && state.jump_hold_ticks > 0 {
        let max_ticks = config.jump_hold_ticks.max(1) as f32;
        let elapsed = (config.jump_hold_ticks - state.jump_hold_ticks) as f32;
        let t = (elapsed / max_ticks).clamp(0.0, 1.0);
        let relief = config
            .jump_hold_gravity_relief_start
            .lerp(config.jump_hold_gravity_relief_end, t)
            .clamp(0.0, 1.0);
        gravity *= 1.0 - relief;
        state.jump_hold_ticks = state.jump_hold_ticks.saturating_sub(1);
    } else {
        state.jump_hold_ticks = 0;
    }
    state.velocity.y = (state.velocity.y - gravity * dt).max(-config.terminal_fall_speed);
}

fn has_jump_floor(config: &FirstPersonControllerConfig, state: &FirstPersonMotorState) -> bool {
    state.grounded && is_walkable_normal(state.ground_normal, config)
}

fn update_jump_input_latch(
    jump_down: bool,
    config: &FirstPersonControllerConfig,
    state: &mut FirstPersonMotorState,
) -> bool {
    let requested = config.jump_enabled && jump_down && !state.jump_input_down;
    state.jump_input_down = jump_down;
    requested
}

pub fn update_ground_contact(
    hit_ground: bool,
    hit_normal: Vec3,
    sticky_ticks: u8,
    state: &mut FirstPersonMotorState,
) {
    if state.climbing {
        state.grounded = true;
        state.ground_contact_ticks = sticky_ticks;
        return;
    }
    if hit_ground {
        state.grounded = true;
        state.ground_contact_ticks = sticky_ticks;
        state.ground_normal = hit_normal.normalize_or_zero();
        if state.ground_normal == Vec3::ZERO {
            state.ground_normal = Vec3::Y;
        }
        return;
    }

    state.ground_contact_ticks = state.ground_contact_ticks.saturating_sub(1);
    state.grounded = state.ground_contact_ticks > 0;
    if !state.grounded {
        state.ground_normal = Vec3::Y;
    }
}
