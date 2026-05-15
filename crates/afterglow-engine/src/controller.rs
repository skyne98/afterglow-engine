use avian3d::{
    character_controller::prelude::MoveAndSlide,
    prelude::{CustomPositionIntegration, Position, Rotation, SpeculativeMargin},
};
use bevy::prelude::*;

use crate::{
    core::schedule::AfterglowSet,
    input::PlayerCommandQueue,
    network::NetworkPlayerId,
    physics::{PhysicsBody, PhysicsCollider},
};

mod body;
mod camera;
mod camera_motion;
mod commands;
mod physics;
mod source_move;
mod stairs;
mod trace;
mod util;
pub use body::{
    apply_first_person_gravity, clamp_local_speeds_to_actual_stance, input_speed_scale,
    integrate_first_person_input, integrate_first_person_motor, local_move_delta_from_speeds,
    project_move_on_ground, sync_local_speeds_from_velocity, update_ground_contact,
    write_flat_horizontal_velocity_from_delta,
};
pub use camera::{
    CameraEffectKind, FirstPersonCameraConfig, FirstPersonCameraImpulse, FirstPersonCameraRig,
    FirstPersonCameraState, FirstPersonFootstep, FirstPersonHeadOffset,
};
pub use physics::{feet_stable_center_delta, is_step_height_allowed};
pub use trace::{
    FirstPersonCameraTraceFrame, FirstPersonControllerTrace, FirstPersonControllerTraceFrame,
    FirstPersonStepRayTrace, FirstPersonStepRejectReason, FirstPersonStepTrace,
};
use util::flat;

#[derive(Component, Clone, Debug, PartialEq, Reflect)]
pub struct FirstPersonController {
    pub player: NetworkPlayerId,
    pub config: FirstPersonControllerConfig,
}

#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct FirstPersonControllerConfig {
    pub move_x_axis: String,
    pub move_y_axis: String,
    pub look_x_axis: String,
    pub look_y_axis: String,
    pub jump_action: String,
    pub crouch_action: String,
    pub toggle_crouch: bool,
    pub sprint_action: String,
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

type ControllerAuthoringItem<'a> = (
    Entity,
    &'a FirstPersonController,
    Option<&'a FirstPersonMotorState>,
);
type ControllerAuthoringFilter = Or<(Added<FirstPersonController>, Changed<FirstPersonController>)>;

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
            .register_type::<FirstPersonFootstep>()
            .register_type::<CameraEffectKind>()
            .init_resource::<FirstPersonControllerTrace>()
            .add_message::<FirstPersonFootstep>()
            .add_systems(
                Update,
                (
                    sync_first_person_controller_authoring,
                    camera::sync_first_person_camera_rig_authoring,
                    drive_first_person_controllers,
                )
                    .chain()
                    .in_set(AfterglowSet::Simulate),
            )
            .add_systems(
                Update,
                camera::update_first_person_camera_rigs.in_set(AfterglowSet::ApplyGameplay),
            );
    }
}

impl Default for FirstPersonControllerConfig {
    fn default() -> Self {
        Self {
            move_x_axis: "move.x".into(),
            move_y_axis: "move.y".into(),
            look_x_axis: "look.x".into(),
            look_y_axis: "look.y".into(),
            jump_action: "jump".into(),
            crouch_action: "crouch".into(),
            toggle_crouch: false,
            sprint_action: "sprint".into(),
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
    pub fn new(player: NetworkPlayerId) -> Self {
        Self {
            player,
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

fn sync_first_person_controller_authoring(
    mut commands: Commands,
    controllers: Query<ControllerAuthoringItem, ControllerAuthoringFilter>,
) {
    for (entity, controller, state) in &controllers {
        let config = &controller.config;
        let stance = state.map_or(ControllerStance::Standing, |state| state.stance);
        let height = config.height(stance);
        let mut entity_commands = commands.entity(entity);
        entity_commands.insert((
            PhysicsBody::kinematic(),
            PhysicsCollider::cylinder(config.body_radius, height),
            CustomPositionIntegration,
            SpeculativeMargin(0.0),
        ));
        if state.is_none() {
            entity_commands.insert(FirstPersonMotorState::default());
        }
    }
}

fn drive_first_person_controllers(
    mut entity_commands: Commands,
    time: Res<Time>,
    player_commands: Option<Res<PlayerCommandQueue>>,
    mut controllers: Query<(
        Entity,
        &FirstPersonController,
        &mut FirstPersonMotorState,
        &mut Transform,
    )>,
    move_and_slide: MoveAndSlide,
    spatial_query: avian3d::prelude::SpatialQuery,
    mut trace: ResMut<FirstPersonControllerTrace>,
) {
    let dt = time.delta_secs();
    let command_lookup = player_commands
        .as_deref()
        .map(commands::PlayerCommandLookup::new);
    let record_trace = trace.enabled;
    for (entity, controller, mut state, mut transform) in &mut controllers {
        let start_position = transform.translation;
        physics::update_step_climbing(&controller.config, &mut state, dt);
        let after_step_latch_position = transform.translation;
        let command = command_lookup
            .as_ref()
            .and_then(|lookup| lookup.get(controller.player));
        let input = integrate_first_person_input(command, &controller.config, &mut state, dt);
        let after_input_position = transform.translation;
        let mut active_collider = physics::controller_collider(&controller.config, state.stance);
        if let Some((new_collider, new_authored_collider)) = physics::sync_body_stance(
            entity,
            &controller.config,
            &mut state,
            &mut transform,
            &spatial_query,
        ) {
            active_collider = new_collider.clone();
            entity_commands
                .entity(entity)
                .insert((new_collider, new_authored_collider));
        }
        let after_stance_position = transform.translation;
        clamp_local_speeds_to_actual_stance(&input.command, &controller.config, &mut state);
        transform.rotation = Quat::from_rotation_y(state.yaw);
        let move_delta = local_move_delta_from_speeds(&state, dt);
        let horizontal_pushback = physics::apply_horizontal_move(physics::CharacterMove {
            entity,
            config: &controller.config,
            state: &mut state,
            transform: &mut transform,
            collider: &active_collider,
            move_and_slide: &move_and_slide,
            spatial_query: &spatial_query,
            delta: move_delta,
        });
        let after_horizontal_position = transform.translation;
        let step = stairs::apply_step_attempt(stairs::StepAttempt {
            entity,
            config: &controller.config,
            state: &mut state,
            transform: &mut transform,
            collider: &active_collider,
            spatial_query: &spatial_query,
            desired_delta: move_delta,
            dt,
            record_trace,
        });
        let after_step_position = transform.translation;
        let actual = after_step_position - after_stance_position;
        write_flat_horizontal_velocity_from_delta(&mut state, flat(actual), dt);
        let gravity_applied = !state.climbing && !input.jumped;
        if gravity_applied {
            apply_first_person_gravity(&controller.config, &mut state, dt);
        }
        let vertical_delta = Vec3::Y * state.velocity.y * dt;
        let vertical_pushback = physics::apply_vertical_force_collision(physics::CharacterMove {
            entity,
            config: &controller.config,
            state: &mut state,
            transform: &mut transform,
            collider: &active_collider,
            move_and_slide: &move_and_slide,
            spatial_query: &spatial_query,
            delta: vertical_delta,
        });
        let after_vertical_position = transform.translation;
        physics::probe_ground_normal(
            entity,
            &controller.config,
            &mut state,
            &transform,
            &spatial_query,
        );
        if record_trace {
            trace.push_controller(FirstPersonControllerTraceFrame {
                entity,
                player: controller.player,
                tick: input.command.tick,
                dt,
                command_move: input.command.move_axis,
                command_look: input.command.look_axis,
                jump_down: input.command.jump_down(),
                crouch_pressed: input.command.crouch_pressed,
                sprint_down: input.command.sprint_down(),
                start_position,
                after_step_latch_position,
                after_input_position,
                after_stance_position,
                intended_horizontal_delta: move_delta,
                horizontal_pushback,
                after_horizontal_position,
                step,
                after_step_position,
                gravity_applied,
                vertical_delta,
                vertical_pushback,
                after_vertical_position,
                grounded: state.grounded,
                ground_contact_ticks: state.ground_contact_ticks,
                climbing: state.climbing,
                ground_normal: state.ground_normal,
                local_speed: body::local_speeds_from_velocity(&state),
                velocity: state.velocity,
            });
        }
        sync_physics_transform(entity, &transform, &mut entity_commands);
    }
}

pub(crate) fn is_walkable_normal(normal: Vec3, config: &FirstPersonControllerConfig) -> bool {
    normal.normalize_or(Vec3::Y).dot(Vec3::Y) >= walkable_floor_dot(config)
}

pub(crate) fn walkable_floor_dot(config: &FirstPersonControllerConfig) -> f32 {
    config.max_slope_angle.cos()
}

fn sync_physics_transform(entity: Entity, transform: &Transform, commands: &mut Commands) {
    commands.entity(entity).insert((
        Position(transform.translation),
        Rotation(transform.rotation),
    ));
}

#[cfg(test)]
mod authoring_tests;
#[cfg(test)]
mod blocker_tests;
#[cfg(test)]
mod crouch_terrain_tests;
#[cfg(test)]
mod integration_tests;
#[cfg(test)]
mod jump_tests;
#[cfg(test)]
mod physics_tests;
#[cfg(test)]
mod terrain_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod trace_tests;
