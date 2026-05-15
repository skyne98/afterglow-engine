use avian3d::{
    dynamics::rigid_body::forces::{ConstantForce, ConstantTorque},
    prelude::{AngularVelocity, LinearVelocity, Mass},
};
use bevy::prelude::*;

use super::{
    ActiveInteraction, InteractionKind, PlayerInteractionState,
    pid::PidController,
};

const GRAB_FORCE_P: f32 = 400.0;
const GRAB_FORCE_D: f32 = 40.0;
const GRAB_TORQUE_P: f32 = 40.0;
const GRAB_TORQUE_D: f32 = 0.4;
const GRAB_MAX_FORCE: f32 = 1000.0;
const GRAB_MAX_TORQUE: f32 = 1000.0;
const GRAB_ERROR_WINDOW: usize = 20;

#[derive(Component, Clone, Debug, Reflect)]
pub struct Grabbed {
    pub by: Entity,
    pub offset: Vec3,
    pub rotation_offset: Quat,
    pub depth: f32,
    pub saved_gravity: bool,
    pub saved_mass: f32,
    pub saved_mass_entity: Entity,
}

#[derive(Resource, Clone, Debug)]
pub struct GrabConfig {
    pub grab_force_p: f32,
    pub grab_force_d: f32,
    pub grab_torque_p: f32,
    pub grab_torque_d: f32,
    pub max_force: f32,
    pub max_torque: f32,
    pub grab_deactivate_distance: f32,
}

impl Default for GrabConfig {
    fn default() -> Self {
        Self {
            grab_force_p: GRAB_FORCE_P,
            grab_force_d: GRAB_FORCE_D,
            grab_torque_p: GRAB_TORQUE_P,
            grab_torque_d: GRAB_TORQUE_D,
            max_force: GRAB_MAX_FORCE,
            max_torque: GRAB_MAX_TORQUE,
            grab_deactivate_distance: 3.0,
        }
    }
}

#[derive(Resource, Clone, Debug)]
pub struct GrabPidState {
    pub force_pid: PidController,
    pub torque_pid: PidController,
}

impl Default for GrabPidState {
    fn default() -> Self {
        Self {
            force_pid: PidController::new(GRAB_FORCE_P, 0.0, GRAB_FORCE_D, GRAB_ERROR_WINDOW),
            torque_pid: PidController::new(GRAB_TORQUE_P, 0.0, GRAB_TORQUE_D, GRAB_ERROR_WINDOW),
        }
    }
}

/// Start grabbing: respond to interact press while looking at a grabbable target.
pub fn interact_grab_start(
    mut commands: Commands,
    mut state: ResMut<PlayerInteractionState>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    targets: Query<(&super::InteractionTarget, &Transform)>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    mut grab_state: ResMut<GrabPidState>,
    config: Res<GrabConfig>,
    masses: Query<&Mass>,
) {
    let pressed = keys.just_pressed(KeyCode::KeyE) || mouse.just_pressed(MouseButton::Left);
    if !pressed {
        return;
    }
    let Some(focus_entity) = state.focus_entity else {
        return;
    };
    let Ok((target, transform)) = targets.get(focus_entity) else {
        return;
    };
    let InteractionKind::Grabbable { .. } = target.kind else {
        return;
    };
    let Ok((_camera, camera_transform)) = camera_query.single() else {
        return;
    };

    grab_state.force_pid.reset();
    grab_state.torque_pid.reset();

    let camera_pos = camera_transform.translation();
    let camera_dir = camera_transform.forward();
    let depth = camera_pos.distance(transform.translation).max(config.grab_deactivate_distance * 0.3);
    let offset = transform.translation - (camera_pos + camera_dir * depth);

    let saved_mass = masses.get(focus_entity).map(|m| m.0).unwrap_or(1.0);

    commands.entity(focus_entity).insert((
        Grabbed {
            by: Entity::PLACEHOLDER,
            offset,
            rotation_offset: Quat::IDENTITY,
            depth,
            saved_gravity: true,
            saved_mass,
            saved_mass_entity: focus_entity,
        },
        // Start with zero forces — they will be updated each frame
        ConstantForce::default(),
        ConstantTorque::default(),
    ));

    state.active_interaction = Some(ActiveInteraction::Grabbing {
        entity: focus_entity,
        depth,
        body_offset: offset,
        body_rotation_offset: Quat::IDENTITY,
    });
}

/// Every frame: update ConstantForce/ConstantTorque via PIDs to track the camera goal.
pub fn update_grabbed_objects(
    mut state: ResMut<PlayerInteractionState>,
    mut grab_state: ResMut<GrabPidState>,
    config: Res<GrabConfig>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    mut grabbed_query: Query<(
        Entity,
        &mut Grabbed,
        &Transform,
        &AngularVelocity,
        &mut LinearVelocity,
    )>,
    mut forces: Query<&mut ConstantForce>,
    mut torques: Query<&mut ConstantTorque>,
    masses: Query<&Mass>,
    time: Res<Time>,
) {
    let Some(ActiveInteraction::Grabbing {
        entity,
        depth,
        body_offset,
        body_rotation_offset: _,
    }) = &state.active_interaction
    else {
        return;
    };

    let Ok((_camera, camera_transform)) = camera_query.single() else {
        return;
    };
    let Ok((_grabbed_entity, mut grabbed, transform, angvel, mut linvel)) =
        grabbed_query.get_mut(*entity)
    else {
        state.active_interaction = None;
        return;
    };
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    // ----- LINEAR FORCE -----
    // Goal position = camera + forward*depth + rotated offset
    let goal_pos = camera_transform.translation()
        + camera_transform.forward() * *depth
        + camera_transform.rotation() * *body_offset;
    let pos_error = goal_pos - transform.translation;
    let force_mag = grab_state
        .force_pid
        .output(pos_error.length(), dt)
        .clamp(0.0, config.max_force);

    if force_mag > 0.0 {
        let force_dir = pos_error / pos_error.length();
        let mass = masses.get(*entity).map(|m| m.0).unwrap_or(1.0);
        if let Ok(mut cf) = forces.get_mut(*entity) {
            cf.0 = force_dir * force_mag * mass;
        }
    }

    // ----- ANGULAR TORQUE -----
    // Align body orientation to the camera rotation (plus stored offset)
    let goal_rot = camera_transform.rotation() * grabbed.rotation_offset;
    let rot_diff = goal_rot * transform.rotation.inverse();
    let (rot_axis, rot_angle) = rot_diff.to_axis_angle();
    if rot_angle.abs() > 0.001 {
        let wanted_angvel = rot_axis * rot_angle * 100.0;
        let angvel_error = wanted_angvel - angvel.0;
        let torque_mag = grab_state
            .torque_pid
            .output(angvel_error.length(), dt)
            .clamp(0.0, config.max_torque);
        if torque_mag > 0.0 {
            let torque_dir = angvel_error / angvel_error.length();
            if let Ok(mut ct) = torques.get_mut(*entity) {
                ct.0 = torque_dir * torque_mag;
            }
        }
    }

    // Zero linear velocity so it doesn't fight the force PID
    linvel.0 = Vec3::ZERO;
    grabbed.depth = *depth;
}

/// Release the grab when interact (E / LMB) is released.
pub fn release_grabbed_on_interact_release(
    mut commands: Commands,
    mut state: ResMut<PlayerInteractionState>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
) {
    let released = keys.just_released(KeyCode::KeyE) || mouse.just_released(MouseButton::Left);
    if !released {
        return;
    }
    if let Some(ActiveInteraction::Grabbing { entity, .. }) = &state.active_interaction {
        commands.entity(*entity).remove::<Grabbed>();
        commands.entity(*entity).remove::<ConstantForce>();
        commands.entity(*entity).remove::<ConstantTorque>();
        state.active_interaction = None;
    }
}

/// Release if the grabbed object gets too far from the camera.
pub fn release_distant_grabbed_objects(
    mut commands: Commands,
    mut state: ResMut<PlayerInteractionState>,
    config: Res<GrabConfig>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    grabbed_query: Query<(Entity, &Grabbed, &Transform)>,
) {
    let Some(ActiveInteraction::Grabbing { entity, .. }) = &state.active_interaction else {
        return;
    };
    let Ok((_camera, camera_transform)) = camera_query.single() else {
        return;
    };
    let Ok((_grabbed_entity, _grabbed, transform)) = grabbed_query.get(*entity) else {
        state.active_interaction = None;
        return;
    };
    let dist = camera_transform.translation().distance(transform.translation);
    if dist > config.grab_deactivate_distance {
        commands.entity(*entity).remove::<Grabbed>();
        commands.entity(*entity).remove::<ConstantForce>();
        commands.entity(*entity).remove::<ConstantTorque>();
        state.active_interaction = None;
    }
}

/// Throw the grabbed object forward (right-click).
pub fn throw_grabbed_object(
    mut commands: Commands,
    mut state: ResMut<PlayerInteractionState>,
    mouse: Res<ButtonInput<MouseButton>>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    mut grabbed_query: Query<(Entity, &mut LinearVelocity)>,
    targets: Query<&super::InteractionTarget>,
) {
    if !mouse.just_pressed(MouseButton::Right) {
        return;
    }
    let Some(ActiveInteraction::Grabbing { entity, .. }) = &state.active_interaction else {
        return;
    };
    let Ok((_camera, camera_transform)) = camera_query.single() else {
        return;
    };
    let Ok((grabbed_entity, mut linvel)) = grabbed_query.get_mut(*entity) else {
        return;
    };

    let throw_impulse = targets
        .get(grabbed_entity)
        .ok()
        .and_then(|t| {
            if let InteractionKind::Grabbable {
                throw_impulse, ..
            } = t.kind
            {
                Some(throw_impulse)
            } else {
                None
            }
        })
        .unwrap_or(10.0);

    linvel.0 = camera_transform.forward() * throw_impulse;
    commands.entity(grabbed_entity).remove::<Grabbed>();
    commands.entity(grabbed_entity).remove::<ConstantForce>();
    commands.entity(grabbed_entity).remove::<ConstantTorque>();
    state.active_interaction = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grab_config_defaults_are_sane() {
        let config = GrabConfig::default();
        assert!((config.grab_force_p - 400.0).abs() < f32::EPSILON);
        assert!((config.grab_force_d - 40.0).abs() < f32::EPSILON);
        assert!((config.grab_torque_p - 40.0).abs() < f32::EPSILON);
        assert!((config.grab_torque_d - 0.4).abs() < f32::EPSILON);
        assert!((config.max_force - 1000.0).abs() < f32::EPSILON);
        assert!((config.max_torque - 1000.0).abs() < f32::EPSILON);
        assert!((config.grab_deactivate_distance - 3.0).abs() < f32::EPSILON);
    }

    #[test]
    fn grab_pid_state_resets() {
        let mut state = GrabPidState::default();
        state.force_pid.output(10.0, 1.0);
        state.torque_pid.output(5.0, 1.0);
        state.force_pid.reset();
        state.torque_pid.reset();
        assert!((state.force_pid.output(0.0, 1.0) - 0.0).abs() < 0.001);
        assert!((state.torque_pid.output(0.0, 1.0) - 0.0).abs() < 0.001);
    }

    #[test]
    fn grabbed_component_holds_expected_data() {
        let grabbed = Grabbed {
            by: Entity::PLACEHOLDER,
            offset: Vec3::new(0.0, 0.0, -0.5),
            rotation_offset: Quat::IDENTITY,
            depth: 1.5,
            saved_gravity: true,
            saved_mass: 10.0,
            saved_mass_entity: Entity::PLACEHOLDER,
        };
        assert!((grabbed.depth - 1.5).abs() < f32::EPSILON);
        assert!((grabbed.saved_mass - 10.0).abs() < f32::EPSILON);
    }
}
