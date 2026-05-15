use bevy::prelude::*;

use super::{
    ControllerStance, FirstPersonCameraTraceFrame, FirstPersonController,
    FirstPersonControllerTrace, FirstPersonMotorState,
    body::local_speeds_from_velocity,
    camera_motion::{
        advance_hpl2_bob_phase_to_rest, hpl2_bob_reached_rest, hpl2_bob_step_crossed,
        hpl2_head_bob, hpl2_landing_bounce, move_scalar_toward_slowdown, move_vec2_toward, smooth,
        smooth_vec3,
    },
};

#[derive(Component, Clone, Debug, PartialEq, Reflect)]
pub struct FirstPersonCameraRig {
    pub target: Entity,
    pub config: FirstPersonCameraConfig,
}

#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct FirstPersonCameraConfig {
    pub standing_eye_height: f32,
    pub crouching_eye_height: f32,
    pub position_smooth_speed: f32,
    pub crouch_down_head_speed: f32,
    pub stand_up_head_speed: f32,
    pub crouch_head_slow_distance: f32,
    pub walk_bob_amplitude: Vec2,
    pub run_bob_amplitude: Vec2,
    pub crouch_bob_amplitude: Vec2,
    pub walk_bob_min_speed: f32,
    pub walk_bob_max_speed: f32,
    pub run_bob_min_speed: f32,
    pub run_bob_max_speed: f32,
    pub crouch_bob_min_speed: f32,
    pub crouch_bob_max_speed: f32,
    pub bob_blend_speed: f32,
    pub ground_bounce_size: f32,
    pub ground_bounce_speed: f32,
    pub min_hit_ground_bounce_speed: f32,
    pub fov: f32,
    pub sprint_fov_add: f32,
    pub fov_smooth_speed: f32,
    pub impulse_decay: f32,
    pub head_offset_smooth_speed: f32,
}

#[derive(Component, Clone, Debug, PartialEq, Reflect)]
pub struct FirstPersonCameraState {
    pub initialized: bool,
    pub smoothed_position: Vec3,
    pub eye_height: f32,
    pub bob_phase: f32,
    pub bobbing: bool,
    pub current_bob_amplitude: Vec2,
    pub landing_bounce: f32,
    pub landing_bounce_phase: f32,
    pub landing_bounce_mul: f32,
    pub roll: f32,
    pub fov: f32,
    pub was_grounded: bool,
    pub last_vertical_velocity: f32,
    pub impulse_pitch: f32,
    pub impulse_yaw: f32,
    pub impulse_roll: f32,
    pub head_offset: Vec3,
    pub footstep_count: u64,
}

#[derive(Component, Clone, Debug, PartialEq, Reflect)]
pub struct FirstPersonHeadOffset {
    pub kind: CameraEffectKind,
    pub offset: Vec3,
    pub weight: f32,
}

#[derive(Component, Clone, Debug, PartialEq, Reflect)]
pub struct FirstPersonCameraImpulse {
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Message, Reflect)]
pub struct FirstPersonFootstep {
    pub target: Entity,
    pub speed: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Reflect)]
pub enum CameraEffectKind {
    Crouch,
    Interaction,
    Damage,
    Script,
    Horror,
}

impl FirstPersonCameraRig {
    pub fn new(target: Entity) -> Self {
        Self {
            target,
            config: FirstPersonCameraConfig::default(),
        }
    }
}

impl Default for FirstPersonCameraConfig {
    fn default() -> Self {
        Self {
            standing_eye_height: 1.58,
            crouching_eye_height: 0.95,
            position_smooth_speed: 28.0,
            crouch_down_head_speed: 3.0,
            stand_up_head_speed: 3.6,
            crouch_head_slow_distance: 0.05,
            walk_bob_amplitude: Vec2::new(0.03, 0.03),
            run_bob_amplitude: Vec2::new(0.05, 0.06),
            crouch_bob_amplitude: Vec2::new(0.06, 0.04),
            walk_bob_min_speed: 0.4,
            walk_bob_max_speed: 1.8,
            run_bob_min_speed: 0.5,
            run_bob_max_speed: 2.5,
            crouch_bob_min_speed: 0.2,
            crouch_bob_max_speed: 1.2,
            bob_blend_speed: 0.1,
            ground_bounce_size: 0.08,
            ground_bounce_speed: 2.8,
            min_hit_ground_bounce_speed: 5.0,
            fov: 70.0_f32.to_radians(),
            sprint_fov_add: 4.0_f32.to_radians(),
            fov_smooth_speed: 8.0,
            impulse_decay: 7.0,
            head_offset_smooth_speed: 10.0,
        }
    }
}

impl Default for FirstPersonCameraState {
    fn default() -> Self {
        Self {
            initialized: false,
            smoothed_position: Vec3::ZERO,
            eye_height: 1.58,
            bob_phase: 0.0,
            bobbing: false,
            current_bob_amplitude: Vec2::ZERO,
            landing_bounce: 0.0,
            landing_bounce_phase: 1.0,
            landing_bounce_mul: 1.0,
            roll: 0.0,
            fov: 70.0_f32.to_radians(),
            was_grounded: false,
            last_vertical_velocity: 0.0,
            impulse_pitch: 0.0,
            impulse_yaw: 0.0,
            impulse_roll: 0.0,
            head_offset: Vec3::ZERO,
            footstep_count: 0,
        }
    }
}

pub fn update_first_person_camera_rigs(
    time: Res<Time>,
    targets: Query<
        (&Transform, &FirstPersonController, &FirstPersonMotorState),
        Without<FirstPersonCameraRig>,
    >,
    offsets: Query<(&ChildOf, &FirstPersonHeadOffset)>,
    mut impulses: Query<(Entity, &ChildOf, &FirstPersonCameraImpulse)>,
    mut commands: Commands,
    mut footsteps: MessageWriter<FirstPersonFootstep>,
    mut trace: ResMut<FirstPersonControllerTrace>,
    mut cameras: Query<
        (
            Entity,
            &FirstPersonCameraRig,
            &mut FirstPersonCameraState,
            &mut Transform,
            Option<&mut Projection>,
        ),
        Without<FirstPersonController>,
    >,
) {
    let dt = time.delta_secs();
    let record_trace = trace.enabled;
    for (camera, rig, mut state, mut transform, projection) in &mut cameras {
        let Ok((target_transform, controller, motor)) = targets.get(rig.target) else {
            continue;
        };
        let offset = summed_head_offset(rig.target, &offsets);
        consume_impulses(rig.target, &mut state, &mut impulses, &mut commands);
        let footstep_emitted = update_camera_state(
            &rig.config,
            &mut state,
            target_transform,
            controller,
            motor,
            offset,
            dt,
        );
        if footstep_emitted {
            footsteps.write(FirstPersonFootstep {
                target: rig.target,
                speed: local_speeds_from_velocity(motor).length(),
            });
        }
        let bob_offset = apply_camera_transform(&state, motor, &mut transform);
        if record_trace {
            trace.push_camera(FirstPersonCameraTraceFrame {
                camera,
                target: rig.target,
                dt,
                base_position: state.smoothed_position,
                bob_offset,
                final_position: transform.translation,
                bobbing: state.bobbing,
                bob_phase: state.bob_phase,
                current_bob_amplitude: state.current_bob_amplitude,
                landing_bounce: state.landing_bounce,
                footstep_emitted,
            });
        }
        if let Some(projection) = projection {
            apply_camera_fov(&state, projection);
        }
    }
}

pub fn sync_first_person_camera_rig_authoring(
    mut commands: Commands,
    rigs: Query<
        (
            Entity,
            &FirstPersonCameraRig,
            Option<&FirstPersonCameraState>,
        ),
        Added<FirstPersonCameraRig>,
    >,
) {
    for (entity, rig, state) in &rigs {
        if state.is_none() {
            commands.entity(entity).insert(FirstPersonCameraState {
                eye_height: rig.config.standing_eye_height,
                fov: rig.config.fov,
                ..default()
            });
        }
    }
}

fn summed_head_offset(target: Entity, offsets: &Query<(&ChildOf, &FirstPersonHeadOffset)>) -> Vec3 {
    offsets
        .iter()
        .filter(|(parent, _)| parent.parent() == target)
        .map(|(_, offset)| offset.offset * offset.weight)
        .sum()
}

fn consume_impulses(
    target: Entity,
    state: &mut FirstPersonCameraState,
    impulses: &mut Query<(Entity, &ChildOf, &FirstPersonCameraImpulse)>,
    commands: &mut Commands,
) {
    for (entity, parent, impulse) in impulses.iter_mut() {
        if parent.parent() != target {
            continue;
        }
        state.impulse_pitch += impulse.pitch;
        state.impulse_yaw += impulse.yaw;
        state.impulse_roll += impulse.roll;
        commands.entity(entity).despawn();
    }
}

fn update_camera_state(
    config: &FirstPersonCameraConfig,
    state: &mut FirstPersonCameraState,
    target_transform: &Transform,
    controller: &FirstPersonController,
    motor: &FirstPersonMotorState,
    target_offset: Vec3,
    dt: f32,
) -> bool {
    let target_eye_height = eye_height(config, motor.stance);
    let eye_speed = if target_eye_height < state.eye_height {
        config.crouch_down_head_speed
    } else {
        config.stand_up_head_speed
    };
    state.eye_height = move_scalar_toward_slowdown(
        state.eye_height,
        target_eye_height,
        eye_speed,
        config.crouch_head_slow_distance,
        dt,
    );
    state.head_offset = smooth_vec3(
        state.head_offset,
        target_offset,
        config.head_offset_smooth_speed,
        dt,
    );

    let local_speed = local_speeds_from_velocity(motor);
    let moving = motor.grounded && local_speed.length() > 0.05;
    let target_bob_amplitude = if moving {
        bob_amplitude(config, motor, controller, local_speed)
    } else {
        Vec2::ZERO
    };
    state.current_bob_amplitude = move_vec2_toward(
        state.current_bob_amplitude,
        target_bob_amplitude,
        config.bob_blend_speed * dt,
    );
    let previous_bob_phase = state.bob_phase;
    if moving {
        state.bobbing = true;
        state.bob_phase +=
            bob_speed(config, motor, controller, local_speed) * std::f32::consts::TAU * dt;
    } else if state.bobbing {
        state.bob_phase = advance_hpl2_bob_phase_to_rest(state.bob_phase, dt);
        if hpl2_bob_reached_rest(previous_bob_phase, state.bob_phase) {
            state.bobbing = false;
            state.bob_phase = 0.0;
            state.current_bob_amplitude = Vec2::ZERO;
        }
    }
    let footstep = hpl2_bob_step_crossed(previous_bob_phase, state.bob_phase, moving);
    if footstep {
        state.footstep_count = state.footstep_count.saturating_add(1);
    }

    let was_grounded = state.was_grounded;
    if !was_grounded && motor.grounded {
        let impact = (-state.last_vertical_velocity).max(0.0);
        if impact > config.min_hit_ground_bounce_speed {
            let t = ((impact / config.min_hit_ground_bounce_speed - 1.0) / 1.5).clamp(0.0, 1.0);
            state.landing_bounce_phase = 0.0;
            state.landing_bounce_mul = 1.0 + t * 0.75;
        }
    }
    if state.landing_bounce_phase < 1.0 {
        state.landing_bounce_phase += dt * config.ground_bounce_speed;
        if state.landing_bounce_phase >= 1.0 {
            state.landing_bounce = 0.0;
        } else {
            state.landing_bounce = hpl2_landing_bounce(
                state.landing_bounce_phase,
                config.ground_bounce_size * state.landing_bounce_mul,
            );
        }
    }
    state.last_vertical_velocity = motor.velocity.y;
    state.was_grounded = motor.grounded;

    state.fov = smooth(
        state.fov,
        target_fov(config, controller, local_speed),
        config.fov_smooth_speed,
        dt,
    );
    decay_impulses(state, config.impulse_decay, dt);

    let body_half_height = controller.config.height(motor.stance) * 0.5;
    let target_position = target_transform.translation
        + Vec3::Y * (state.eye_height - body_half_height)
        + state.head_offset;
    state.smoothed_position = if !state.initialized {
        state.initialized = true;
        target_position
    } else if motor.grounded && was_grounded {
        target_position
    } else {
        let smoothed = smooth_vec3(
            state.smoothed_position,
            target_position,
            config.position_smooth_speed,
            dt,
        );
        Vec3::new(target_position.x, smoothed.y, target_position.z)
    };
    footstep
}

fn apply_camera_transform(
    state: &FirstPersonCameraState,
    motor: &FirstPersonMotorState,
    transform: &mut Transform,
) -> Vec3 {
    let bob = hpl2_head_bob(
        state.bobbing,
        state.bob_phase,
        state.current_bob_amplitude,
        state.landing_bounce,
    );
    let rotation = Quat::from_rotation_y(motor.yaw + state.impulse_yaw)
        * Quat::from_rotation_x(motor.pitch + state.impulse_pitch)
        * Quat::from_rotation_z(state.roll + state.impulse_roll);
    let bob_offset = rotation * bob;
    transform.translation = state.smoothed_position + bob_offset;
    transform.rotation = rotation;
    bob_offset
}

fn apply_camera_fov(state: &FirstPersonCameraState, mut projection: Mut<Projection>) {
    if let Projection::Perspective(perspective) = projection.as_mut() {
        perspective.fov = state.fov;
    }
}

fn eye_height(config: &FirstPersonCameraConfig, stance: ControllerStance) -> f32 {
    match stance {
        ControllerStance::Standing => config.standing_eye_height,
        ControllerStance::Crouching => config.crouching_eye_height,
    }
}

fn bob_amplitude(
    config: &FirstPersonCameraConfig,
    motor: &FirstPersonMotorState,
    controller: &FirstPersonController,
    local_speed: Vec2,
) -> Vec2 {
    if motor.stance == ControllerStance::Crouching {
        config.crouch_bob_amplitude
    } else if is_running(controller, local_speed) {
        config.run_bob_amplitude
    } else {
        config.walk_bob_amplitude
    }
}

fn bob_speed(
    config: &FirstPersonCameraConfig,
    motor: &FirstPersonMotorState,
    controller: &FirstPersonController,
    local_speed: Vec2,
) -> f32 {
    let (min_speed, max_speed) = if motor.stance == ControllerStance::Crouching {
        (config.crouch_bob_min_speed, config.crouch_bob_max_speed)
    } else if is_running(controller, local_speed) {
        (config.run_bob_min_speed, config.run_bob_max_speed)
    } else {
        (config.walk_bob_min_speed, config.walk_bob_max_speed)
    };
    let speed = local_speed.length();
    let max_player_speed = if motor.stance == ControllerStance::Crouching {
        controller.config.crouch_speed
    } else if is_running(controller, local_speed) {
        controller.config.sprint_speed
    } else {
        controller.config.ground_speed
    }
    .max(f32::EPSILON);
    min_speed + (speed / max_player_speed).clamp(0.0, 1.0) * (max_speed - min_speed)
}

fn target_fov(
    config: &FirstPersonCameraConfig,
    controller: &FirstPersonController,
    local_speed: Vec2,
) -> f32 {
    let sprinting = local_speed.x > controller.config.ground_speed + 0.1;
    config.fov
        + if sprinting {
            config.sprint_fov_add
        } else {
            0.0
        }
}

fn is_running(controller: &FirstPersonController, local_speed: Vec2) -> bool {
    local_speed.x > controller.config.ground_speed
}

fn decay_impulses(state: &mut FirstPersonCameraState, decay: f32, dt: f32) {
    state.impulse_pitch = smooth(state.impulse_pitch, 0.0, decay, dt);
    state.impulse_yaw = smooth(state.impulse_yaw, 0.0, decay, dt);
    state.impulse_roll = smooth(state.impulse_roll, 0.0, decay, dt);
}

#[cfg(test)]
#[path = "camera_crouch_tests.rs"]
mod crouch_tests;

#[cfg(test)]
#[path = "camera_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "camera_trace_tests.rs"]
mod trace_tests;
