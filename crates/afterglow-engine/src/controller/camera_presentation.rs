use bevy::{prelude::*, time::Fixed};

use super::{super::camera_motion::hpl2_head_bob, FirstPersonCameraState, FirstPersonMotorState};

pub(super) fn fixed_presentation_offset(
    motor: &FirstPersonMotorState,
    fixed_time: &Time<Fixed>,
) -> Vec3 {
    motor.velocity * fixed_time.delta_secs() * fixed_time.overstep_fraction()
}

pub(in crate::controller) fn apply_camera_transform(
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

pub(super) fn apply_camera_fov(state: &FirstPersonCameraState, mut projection: Mut<Projection>) {
    if let Projection::Perspective(perspective) = projection.as_mut() {
        perspective.fov = state.fov;
    }
}
