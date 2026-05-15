use avian3d::{
    dynamics::{
        joints::{DistanceLimit, JointCollisionDisabled, JointForces, PrismaticJoint},
        rigid_body::{forces::ConstantForce, mass_properties::components::Mass},
    },
    prelude::{LinearVelocity, RigidBody},
};
use bevy::prelude::*;

use super::{pid::PidController, ActiveInteraction, PlayerInteractionState};

#[derive(Component, Clone, Debug, Reflect)]
pub struct PrismaticJointConfig {
    pub axis: Vec3,
    pub limit_min: f32,
    pub limit_max: f32,
    pub state_count: Option<usize>,
    pub move_max_speed: f32,
    pub move_slow_down_factor: f32,
    pub move_speed_factor: f32,
    pub move_throw_impulse: f32,
}

impl PrismaticJointConfig {
    pub fn new_drawer(axis: Vec3, length: f32) -> Self {
        Self {
            axis,
            limit_min: 0.0,
            limit_max: length,
            state_count: None,
            move_max_speed: 8.0,
            move_slow_down_factor: 3.0,
            move_speed_factor: 1.0,
            move_throw_impulse: 4.0,
        }
    }

    pub fn new_multi_slider(axis: Vec3, length: f32, states: usize) -> Self {
        Self {
            axis,
            limit_min: 0.0,
            limit_max: length,
            state_count: Some(states),
            move_max_speed: 8.0,
            move_slow_down_factor: 3.0,
            move_speed_factor: 1.0,
            move_throw_impulse: 4.0,
        }
    }
}

#[derive(Component, Clone, Debug, Reflect)]
pub struct PrismaticJointState {
    pub previous_distance: f32,
    pub at_min_limit: bool,
    pub at_max_limit: bool,
}

impl Default for PrismaticJointState {
    fn default() -> Self {
        Self {
            previous_distance: 0.0,
            at_min_limit: false,
            at_max_limit: false,
        }
    }
}

#[derive(Resource, Clone, Debug)]
pub struct DrawerPidState {
    pub speed_pid: PidController,
}

impl Default for DrawerPidState {
    fn default() -> Self {
        Self {
            speed_pid: PidController::new(6.0, 0.0, 0.1, 10),
        }
    }
}

const DRAWER_FORCE_SCALE: f32 = 850.0;

pub fn interact_drawer_system(
    mut commands: Commands,
    mut state: ResMut<PlayerInteractionState>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    interacted: Query<(Entity, &PrismaticJointConfig, &PrismaticJointState)>,
    rigid_bodies: Query<(&RigidBody, &Mass)>,
    linvels: Query<&LinearVelocity>,
    mut pid_state: Local<DrawerPidState>,
    time: Res<Time>,
) {
    let pressed = keys.just_pressed(KeyCode::KeyE) || mouse.just_pressed(MouseButton::Left);

    if pressed {
        if let Some(focus) = state.focus_entity {
            if let Ok((entity, config, _)) = interacted.get(focus) {
                let parent = find_drawer_parent(entity);
                let joint_entity = commands
                    .spawn(create_drawer_joint(parent, entity, config))
                    .insert(JointCollisionDisabled)
                    .id();
                pid_state.speed_pid.reset();
                state.active_interaction = Some(ActiveInteraction::SlidingDrawer {
                    entity,
                    joint_entity,
                    slide_speed: 0.0,
                });
                return;
            }
        }
    }

    if let Some(ActiveInteraction::SlidingDrawer {
        entity,
        joint_entity,
        slide_speed,
    }) = &mut state.active_interaction
    {
        let held = keys.pressed(KeyCode::KeyE) || mouse.pressed(MouseButton::Left);

        if !held {
            commands.entity(*entity).remove::<ConstantForce>();
            commands.entity(*joint_entity).despawn();
            state.active_interaction = None;
            return;
        }

        if let Ok((_, config, _)) = interacted.get(*entity) {
            if let Ok((body, mass)) = rigid_bodies.get(*entity) {
                if !matches!(body, RigidBody::Dynamic) {
                    return;
                }
                let dt = time.delta_secs();
                let mut speed = *slide_speed;
                speed -= speed.signum() * speed.abs() * config.move_slow_down_factor * dt;
                speed += DRAWER_FORCE_SCALE * config.move_speed_factor * dt;
                speed = speed.clamp(-config.move_max_speed, config.move_max_speed);
                *slide_speed = speed;

                // HPL2: PID(wanted_vel - actual_vel) → force * mass
                let actual_vel = linvels
                    .get(*entity)
                    .map(|v| v.0.dot(config.axis))
                    .unwrap_or(0.0);
                let vel_error = speed - actual_vel;
                let force_mag = pid_state.speed_pid.output(vel_error, dt);
                let force_vec = config.axis * force_mag * mass.0;

                commands.entity(*entity).insert(ConstantForce(force_vec));
            }
        }
    }
}

pub fn sticky_drawer_limits(
    mut joints: Query<(&PrismaticJoint, &JointForces, &mut PrismaticJointState)>,
    mut forces_query: Query<&mut ConstantForce>,
) {
    for (joint, forces, mut state) in &mut joints {
        let impulse = forces.force().length();
        if impulse > 0.1 {
            let limit = joint.limits.unwrap_or(DistanceLimit::ZERO);
            let current_distance = 0.0;
            if current_distance <= limit.min + 0.01 {
                state.at_min_limit = true;
                if let Ok(mut f) = forces_query.get_mut(joint.body2) {
                    f.0 = Vec3::ZERO;
                }
            } else {
                state.at_min_limit = false;
            }

            if current_distance >= limit.max - 0.01 {
                state.at_max_limit = true;
                if let Ok(mut f) = forces_query.get_mut(joint.body2) {
                    f.0 = Vec3::ZERO;
                }
            } else {
                state.at_max_limit = false;
            }
        }
    }
}

fn create_drawer_joint(
    parent: Entity,
    child: Entity,
    config: &PrismaticJointConfig,
) -> PrismaticJoint {
    let axis = config.axis.normalize_or(Vec3::X);
    PrismaticJoint::new(parent, child)
        .with_slider_axis(axis)
        .with_limits(config.limit_min, config.limit_max)
}

fn find_drawer_parent(_entity: Entity) -> Entity {
    _entity
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prismatic_joint_config_new_drawer_has_sane_defaults() {
        let config = PrismaticJointConfig::new_drawer(Vec3::X, 0.5);
        assert!(config.axis.distance(Vec3::X) < f32::EPSILON);
        assert!((config.limit_min - 0.0).abs() < f32::EPSILON);
        assert!((config.limit_max - 0.5).abs() < f32::EPSILON);
        assert!(config.state_count.is_none());
    }

    #[test]
    fn drawer_pid_state_defaults() {
        let state = DrawerPidState::default();
        assert!((state.speed_pid.p - 6.0).abs() < f32::EPSILON);
        assert!((state.speed_pid.d - 0.1).abs() < f32::EPSILON);
    }

    #[test]
    fn prismatic_joint_state_defaults() {
        let state = PrismaticJointState::default();
        assert!((state.previous_distance - 0.0).abs() < f32::EPSILON);
        assert!(!state.at_min_limit);
        assert!(!state.at_max_limit);
    }

    #[test]
    fn prismatic_joint_config_multi_slider_has_states() {
        let config = PrismaticJointConfig::new_multi_slider(Vec3::X, 0.6, 5);
        assert_eq!(config.state_count, Some(5));
        assert!((config.limit_max - 0.6).abs() < f32::EPSILON);
    }
}
