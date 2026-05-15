use avian3d::{
    dynamics::{
        joints::{AngleLimit, JointCollisionDisabled, JointForces, RevoluteJoint},
        rigid_body::forces::ConstantTorque,
    },
    prelude::{AngularVelocity, RigidBody},
};
use bevy::prelude::*;

use super::{pid::PidController, ActiveInteraction, PlayerInteractionState};

#[derive(Component, Clone, Debug, Reflect)]
pub struct HingeJointConfig {
    pub axis: Vec3,
    pub limit_min: f32,
    pub limit_max: f32,
    pub auto_close_angle: Option<f32>,
    pub break_impulse: Option<f32>,
    pub move_max_speed: f32,
    pub move_slow_down_factor: f32,
    pub move_speed_factor: f32,
    pub move_throw_impulse: f32,
}

impl HingeJointConfig {
    pub fn new_door(axis: Vec3) -> Self {
        Self {
            axis,
            limit_min: -120.0_f32.to_radians(),
            limit_max: 120.0_f32.to_radians(),
            auto_close_angle: Some(10.0_f32.to_radians()),
            break_impulse: None,
            move_max_speed: 13.5,
            move_slow_down_factor: 3.0,
            move_speed_factor: 1.0,
            move_throw_impulse: 6.0,
        }
    }
}

#[derive(Component, Clone, Debug, Reflect)]
pub struct HingeJointState {
    pub previous_angle: f32,
    pub at_min_limit: bool,
    pub at_max_limit: bool,
}

impl Default for HingeJointState {
    fn default() -> Self {
        Self {
            previous_angle: 0.0,
            at_min_limit: false,
            at_max_limit: false,
        }
    }
}

#[derive(Resource, Clone, Debug)]
pub struct DoorPidState {
    pub rotate_pid: PidController,
}

impl Default for DoorPidState {
    fn default() -> Self {
        Self {
            rotate_pid: PidController::new(10.0, 0.0, 1.0, 10),
        }
    }
}

const DOOR_TORQUE_SCALE: f32 = 3000.0;

pub fn interact_door_system(
    mut commands: Commands,
    mut state: ResMut<PlayerInteractionState>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    interacted: Query<(Entity, &HingeJointConfig, &HingeJointState)>,
    joints: Query<&RevoluteJoint>,
    rigid_bodies: Query<&RigidBody>,
    angvels: Query<&AngularVelocity>,
    mut pid_state: Local<DoorPidState>,
    time: Res<Time>,
) {
    let pressed = keys.just_pressed(KeyCode::KeyE) || mouse.just_pressed(MouseButton::Left);

    if pressed {
        if let Some(focus) = state.focus_entity {
            if let Ok((entity, config, _)) = interacted.get(focus) {
                let parent = find_door_parent(entity);
                let joint_entity = commands
                    .spawn(create_door_joint(parent, entity, config))
                    .insert(JointCollisionDisabled)
                    .id();
                pid_state.rotate_pid.reset();
                state.active_interaction = Some(ActiveInteraction::PushingDoor {
                    entity,
                    joint_entity,
                    rot_speed: 0.0,
                });
                return;
            }
        }
    }

    if let Some(ActiveInteraction::PushingDoor {
        entity,
        joint_entity,
        rot_speed,
    }) = &mut state.active_interaction
    {
        let held = keys.pressed(KeyCode::KeyE) || mouse.pressed(MouseButton::Left);

        if !held {
            if let Ok(joint) = joints.get(*joint_entity) {
                if let Some(_impulse) = interacted.get(*entity).ok().and_then(|(_, c, _)| c.break_impulse) {
                    if rigid_bodies.get(joint.body2).is_ok_and(|b| matches!(b, RigidBody::Dynamic)) {
                    }
                }
            }
            commands.entity(*entity).remove::<ConstantTorque>();
            commands.entity(*joint_entity).despawn();
            state.active_interaction = None;
            return;
        }

        if let Ok((_, config, _)) = interacted.get(*entity) {
            if rigid_bodies.get(*entity).is_ok_and(|b| matches!(b, RigidBody::Dynamic)) {
                let dt = time.delta_secs();
                let mut speed = *rot_speed;
                speed -= speed.signum() * speed.abs() * config.move_slow_down_factor * dt;
                speed += mouse_delta_to_speed() * config.move_speed_factor * dt;
                speed = speed.clamp(-config.move_max_speed, config.move_max_speed);
                *rot_speed = speed;

                // HPL2: PID(wanted_angvel - actual_angvel) → torque * inertia
                let actual_angvel = angvels.get(*entity).map(|a| a.dot(config.axis)).unwrap_or(0.0);
                let angvel_error = speed - actual_angvel;
                let torque = pid_state.rotate_pid.output(angvel_error, dt);
                let torque_vec = config.axis * torque * DOOR_TORQUE_SCALE;

                commands.entity(*entity).insert(ConstantTorque(torque_vec));
            }
        }
    }
}

pub fn sticky_door_limits(
    mut joints: Query<(&RevoluteJoint, &JointForces, &mut HingeJointState)>,
    mut torques: Query<&mut ConstantTorque>,
) {
    for (joint, forces, mut state) in &mut joints {
        let impulse_mag = forces.force().length();
        if impulse_mag > 0.1 {
            let limit = joint.angle_limit.unwrap_or(AngleLimit::ZERO);
            state.at_min_limit = state.previous_angle <= limit.min;
            state.at_max_limit = state.previous_angle >= limit.max;

            if state.at_min_limit || state.at_max_limit {
                if let Ok(mut t) = torques.get_mut(joint.body2) {
                    t.0 = Vec3::ZERO;
                }
            }
        }
    }
}

fn create_door_joint(parent: Entity, child: Entity, config: &HingeJointConfig) -> RevoluteJoint {
    let axis = config.axis.normalize_or(Vec3::Y);
    RevoluteJoint::new(parent, child)
        .with_hinge_axis(axis)
        .with_angle_limits(config.limit_min, config.limit_max)
}

fn find_door_parent(_entity: Entity) -> Entity {
    _entity
}

fn mouse_delta_to_speed() -> f32 {
    3000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hinge_joint_config_new_door_has_sane_defaults() {
        let config = HingeJointConfig::new_door(Vec3::Y);
        assert!(config.axis.distance(Vec3::Y) < f32::EPSILON);
        assert!((config.limit_min + 120.0_f32.to_radians()).abs() < 0.001);
        assert!((config.limit_max - 120.0_f32.to_radians()).abs() < 0.001);
        assert_eq!(config.auto_close_angle, Some(10.0_f32.to_radians()));
        assert!((config.move_max_speed - 13.5).abs() < f32::EPSILON);
    }

    #[test]
    fn door_pid_state_defaults() {
        let state = DoorPidState::default();
        assert!((state.rotate_pid.p - 10.0).abs() < f32::EPSILON);
        assert!((state.rotate_pid.d - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn hinge_joint_state_defaults() {
        let state = HingeJointState::default();
        assert!((state.previous_angle - 0.0).abs() < f32::EPSILON);
        assert!(!state.at_min_limit);
        assert!(!state.at_max_limit);
    }
}
