use bevy::prelude::*;

use crate::core::schedule::AfterglowSet;

#[derive(Component, Clone, Debug)]
pub struct PredictionErrorSmoothing {
    pub error: Vec3,
    pub decay_start: f32,
}

impl Default for PredictionErrorSmoothing {
    fn default() -> Self {
        Self {
            error: Vec3::ZERO,
            decay_start: 0.0,
        }
    }
}

mod body;
mod camera;
mod camera_motion;
mod effects;
mod impulse_buffer;
mod physics;
mod source_move;
mod stairs;
mod systems;
mod trace;
mod util;
pub use body::{
    FirstPersonCommandState, FirstPersonInputStep, ReplayCommand, apply_first_person_gravity,
    clamp_local_speeds_to_actual_stance, input_speed_scale, integrate_first_person_command,
    integrate_first_person_command_look, integrate_first_person_input, integrate_first_person_look,
    integrate_first_person_motor, local_move_delta_from_speeds, project_move_on_ground,
    sync_local_speeds_from_velocity, update_ground_contact,
    write_flat_horizontal_velocity_from_delta,
};
pub use camera::{
    CameraEffectKind, FirstPersonCameraConfig, FirstPersonCameraImpulse, FirstPersonCameraRig,
    FirstPersonCameraState, FirstPersonFootstep, FirstPersonHeadOffset,
};
pub use effects::{FirstPersonEffect, FirstPersonEffectStack};
pub use impulse_buffer::FirstPersonImpulseBuffer;
pub use physics::{feet_stable_center_delta, is_step_height_allowed};
use systems::{
    drive_first_person_controllers, sync_first_person_controller_authoring,
    update_first_person_look,
};
pub use trace::{
    FirstPersonCameraTraceFrame, FirstPersonControllerTrace, FirstPersonControllerTraceFrame,
    FirstPersonStepRayTrace, FirstPersonStepRejectReason, FirstPersonStepTrace,
};

#[derive(Component, Clone, Debug, PartialEq, Reflect)]
pub struct FirstPersonController {
    pub config: FirstPersonControllerConfig,
}

#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct FirstPersonControllerConfig {
    pub toggle_crouch: bool,
    pub jump_enabled: bool,
    pub ground_speed: f32,
    pub sprint_speed: f32,
    pub crouch_speed: f32,
    pub backward_speed: f32,
    pub side_speed: f32,
    pub ground_accel: f32,
    pub side_accel: f32,
    pub ground_deaccel: f32,
    pub side_deaccel: f32,
    pub opposite_dir_accel_mul: f32,
    pub side_opposite_dir_accel_mul: f32,
    pub air_wish_speed: f32,
    pub deaccelerate_in_air: bool,
    pub gravity: f32,
    pub jump_speed: f32,
    pub terminal_fall_speed: f32,
    pub look_sensitivity: Vec2,
    pub min_pitch: f32,
    pub max_pitch: f32,
    pub max_slope_angle: f32,
    pub ground_probe_distance: f32,
    pub body_radius: f32,
    pub standing_height: f32,
    pub crouching_height: f32,
    pub min_step_height: f32,
    pub max_step_height: f32,
    pub max_step_height_in_air: f32,
    pub step_check_interval: f32,
    pub step_climb_height_add: f32,
    pub step_climb_speed: f32,
    pub accurate_climbing: bool,
    pub climb_forward_mul: f32,
    pub depenetration_iterations: usize,
    pub skin_width: f32,
    pub coyote_ticks: u8,
    pub jump_buffer_ticks: u8,
    pub ground_sticky_ticks: u8,
    pub jump_hold_ticks: u8,
    pub jump_hold_gravity_relief_start: f32,
    pub jump_hold_gravity_relief_end: f32,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Reflect)]
pub struct FirstPersonMotorState {
    pub velocity: Vec3,
    pub grounded: bool,
    pub ground_normal: Vec3,
    pub stance: ControllerStance,
    pub desired_stance: ControllerStance,
    pub forward_speed: f32,
    pub side_speed: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub coyote_ticks: u8,
    pub jump_buffer_ticks: u8,
    pub ground_contact_ticks: u8,
    pub jump_hold_ticks: u8,
    pub jump_input_down: bool,
    pub step_check_timer: f32,
    pub climbing: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Reflect)]
pub enum ControllerStance {
    #[default]
    Standing,
    Crouching,
}

pub struct AfterglowFirstPersonControllerPlugin;

impl Plugin for AfterglowFirstPersonControllerPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<FirstPersonController>()
            .register_type::<FirstPersonControllerConfig>()
            .register_type::<FirstPersonMotorState>()
            .register_type::<ControllerStance>()
            .register_type::<FirstPersonCameraRig>()
            .register_type::<FirstPersonCameraConfig>()
            .register_type::<FirstPersonCameraState>()
            .register_type::<FirstPersonHeadOffset>()
            .register_type::<FirstPersonCameraImpulse>()
            .register_type::<FirstPersonImpulseBuffer>()
            .register_type::<FirstPersonEffectStack>()
            .register_type::<FirstPersonEffect>()
            .register_type::<FirstPersonFootstep>()
            .register_type::<CameraEffectKind>()
            .init_resource::<FirstPersonControllerTrace>()
            .add_message::<FirstPersonFootstep>()
            .add_systems(
                Update,
                (
                    sync_first_person_controller_authoring,
                    camera::sync_first_person_camera_rig_authoring,
                )
                    .chain()
                    .in_set(AfterglowSet::Simulate),
            )
            .add_systems(
                FixedUpdate,
                drive_first_person_controllers.in_set(AfterglowSet::Simulate),
            )
            .add_systems(
                Update,
                (
                    update_first_person_look,
                    camera::update_first_person_camera_rigs,
                )
                    .chain()
                    .in_set(AfterglowSet::ApplyGameplay),
            );
    }
}

impl Default for FirstPersonControllerConfig {
    fn default() -> Self {
        Self {
            toggle_crouch: false,
            jump_enabled: true,
            ground_speed: 5.0,
            sprint_speed: 7.0,
            crouch_speed: 2.5,
            backward_speed: 3.4,
            side_speed: 4.2,
            ground_accel: 20.0,
            side_accel: 18.0,
            ground_deaccel: 24.0,
            side_deaccel: 26.0,
            opposite_dir_accel_mul: 2.0,
            side_opposite_dir_accel_mul: 2.25,
            air_wish_speed: 2.5,
            deaccelerate_in_air: false,
            gravity: 24.0,
            jump_speed: 7.0,
            terminal_fall_speed: 55.0,
            look_sensitivity: Vec2::new(0.002, 0.002),
            min_pitch: -1.45,
            max_pitch: 1.45,
            max_slope_angle: 50.0_f32.to_radians(),
            ground_probe_distance: 0.08,
            body_radius: 0.35,
            standing_height: 1.8,
            crouching_height: 1.15,
            min_step_height: 0.025,
            max_step_height: 1.8 * 0.2,
            max_step_height_in_air: 1.8 * 0.2,
            step_check_interval: 1.0 / 20.0,
            step_climb_height_add: 0.01,
            step_climb_speed: 1.0,
            accurate_climbing: false,
            climb_forward_mul: 1.0,
            depenetration_iterations: 4,
            skin_width: 0.0,
            coyote_ticks: 5,
            jump_buffer_ticks: 5,
            ground_sticky_ticks: 12,
            jump_hold_ticks: 12,
            jump_hold_gravity_relief_start: 0.9,
            jump_hold_gravity_relief_end: 0.4,
        }
    }
}

impl FirstPersonController {
    pub fn new() -> Self {
        Self {
            config: FirstPersonControllerConfig::default(),
        }
    }
}

impl Default for FirstPersonMotorState {
    fn default() -> Self {
        Self {
            velocity: Vec3::ZERO,
            grounded: false,
            ground_normal: Vec3::Y,
            stance: ControllerStance::Standing,
            desired_stance: ControllerStance::Standing,
            forward_speed: 0.0,
            side_speed: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            coyote_ticks: 0,
            jump_buffer_ticks: 0,
            ground_contact_ticks: 0,
            jump_hold_ticks: 0,
            jump_input_down: false,
            step_check_timer: 0.0,
            climbing: false,
        }
    }
}

impl FirstPersonControllerConfig {
    pub fn height(&self, stance: ControllerStance) -> f32 {
        match stance {
            ControllerStance::Standing => self.standing_height,
            ControllerStance::Crouching => self.crouching_height,
        }
    }
}

pub(crate) fn is_walkable_normal(normal: Vec3, config: &FirstPersonControllerConfig) -> bool {
    normal.normalize_or(Vec3::Y).dot(Vec3::Y) >= walkable_floor_dot(config)
}

pub(crate) fn walkable_floor_dot(config: &FirstPersonControllerConfig) -> f32 {
    config.max_slope_angle.cos()
}

#[cfg(test)]
mod authoring_tests;
#[cfg(test)]
mod blocker_demo_tests;
#[cfg(test)]
mod blocker_side_tests;
#[cfg(test)]
mod blocker_test_support;
#[cfg(test)]
mod blocker_tests;
#[cfg(test)]
mod camera_effect_tests;
#[cfg(test)]
mod crouch_terrain_tests;
#[cfg(test)]
mod effect_stack_tests;
#[cfg(test)]
mod impulse_buffer_tests;
#[cfg(test)]
mod integration_tests;
#[cfg(test)]
mod jump_tests;
#[cfg(test)]
mod physics_tests;
#[cfg(test)]
mod presentation_tests;
#[cfg(test)]
mod stair_sweep_tests;
#[cfg(test)]
mod terrain_tests;
#[cfg(test)]
mod test_input;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod trace_tests;
