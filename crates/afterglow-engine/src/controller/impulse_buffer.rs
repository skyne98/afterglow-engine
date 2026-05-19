use bevy::prelude::*;

use super::FirstPersonMotorState;

#[derive(Component, Clone, Copy, Debug, PartialEq, Reflect)]
pub struct FirstPersonImpulseBuffer {
    pub linear_velocity_delta: Vec3,
    pub max_linear_velocity_delta: f32,
}

impl Default for FirstPersonImpulseBuffer {
    fn default() -> Self {
        Self {
            linear_velocity_delta: Vec3::ZERO,
            max_linear_velocity_delta: 20.0,
        }
    }
}

impl FirstPersonImpulseBuffer {
    pub fn add_linear_impulse(&mut self, velocity_delta: Vec3) {
        if velocity_delta.is_finite() {
            self.linear_velocity_delta += velocity_delta;
        }
    }

    pub fn drain_linear_impulse(&mut self) -> Vec3 {
        let raw = self.linear_velocity_delta;
        self.linear_velocity_delta = Vec3::ZERO;
        if !raw.is_finite() {
            return Vec3::ZERO;
        }
        if self.max_linear_velocity_delta.is_finite() {
            return raw.clamp_length_max(self.max_linear_velocity_delta.max(0.0));
        }
        raw
    }
}

pub(super) fn apply_first_person_linear_impulse(
    state: &mut FirstPersonMotorState,
    velocity_delta: Vec3,
) {
    if velocity_delta == Vec3::ZERO {
        return;
    }
    let horizontal = Vec3::new(velocity_delta.x, 0.0, velocity_delta.z);
    let rotation = Quat::from_rotation_y(state.yaw);
    state.side_speed += horizontal.dot(rotation * Vec3::X);
    state.forward_speed += horizontal.dot(rotation * Vec3::NEG_Z);
    state.velocity.y += velocity_delta.y;
}
