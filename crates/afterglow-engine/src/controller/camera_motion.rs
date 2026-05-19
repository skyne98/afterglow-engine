use bevy::prelude::*;

use super::util;

const HPL2_BOB_REST_PHASE: f32 = std::f32::consts::FRAC_PI_2;

pub fn hpl2_landing_bounce(phase: f32, size: f32) -> f32 {
    if phase < 0.5 {
        (std::f32::consts::PI + phase * std::f32::consts::PI).sin() * size
    } else {
        (smooth_curve((phase - 0.5) * 2.0) - 1.0) * size
    }
}

pub fn hpl2_head_bob(bobbing: bool, phase: f32, amplitude: Vec2, landing_bounce: f32) -> Vec3 {
    if !bobbing {
        return Vec3::new(0.0, landing_bounce, 0.0);
    }
    Vec3::new(
        (phase * 0.5 - std::f32::consts::FRAC_PI_4).sin() * amplitude.x,
        phase.sin() * amplitude.y - amplitude.y + landing_bounce,
        0.0,
    )
}

pub fn advance_hpl2_bob_phase_to_rest(phase: f32, dt: f32) -> f32 {
    let wrapped = wrap_bob_phase_for_rest(phase);
    let direction = if wrapped <= std::f32::consts::FRAC_PI_2 {
        1.0
    } else {
        -1.0
    };
    phase + dt * std::f32::consts::TAU * direction * 3.1
}

pub fn hpl2_bob_reached_rest(previous_phase: f32, current_phase: f32) -> bool {
    let previous_delta = bob_rest_delta(previous_phase);
    let current_delta = bob_rest_delta(current_phase);
    previous_delta == 0.0
        || current_delta == 0.0
        || previous_delta.signum() != current_delta.signum()
}

pub fn hpl2_bob_step_crossed(previous_phase: f32, current_phase: f32, moving: bool) -> bool {
    if !moving {
        return false;
    }
    let previous_index = (previous_phase / std::f32::consts::PI).floor() as i32;
    let current_index = (current_phase / std::f32::consts::PI).floor() as i32;
    if current_index <= previous_index {
        return false;
    }
    ((previous_index + 1)..=current_index).any(|index| index.rem_euclid(2) == 1)
}

pub fn smooth(current: f32, target: f32, speed: f32, dt: f32) -> f32 {
    current + (target - current) * smoothing_factor(speed, dt)
}

pub fn smooth_vec3(current: Vec3, target: Vec3, speed: f32, dt: f32) -> Vec3 {
    current + (target - current) * smoothing_factor(speed, dt)
}

pub fn move_vec2_toward(current: Vec2, target: Vec2, max_delta: f32) -> Vec2 {
    let delta = target - current;
    let distance = delta.length();
    if distance <= max_delta || distance == 0.0 {
        target
    } else {
        current + delta / distance * max_delta
    }
}

pub fn move_scalar_toward_slowdown(
    current: f32,
    target: f32,
    speed: f32,
    slow_distance: f32,
    dt: f32,
) -> f32 {
    let delta = target - current;
    let distance = delta.abs();
    if distance <= 0.001 {
        return target;
    }
    let slow_distance = slow_distance.max(f32::EPSILON);
    let speed = if distance < slow_distance {
        speed * distance / slow_distance
    } else {
        speed
    };
    if speed * dt >= distance {
        return target;
    }
    current + delta.signum() * speed * dt
}

fn smooth_curve(t: f32) -> f32 {
    util::smoothstep(t)
}

fn wrap_bob_phase_for_rest(phase: f32) -> f32 {
    let min = -std::f32::consts::FRAC_PI_2;
    let max = std::f32::consts::PI * 1.5;
    (phase - min).rem_euclid(max - min) + min
}

fn bob_rest_delta(phase: f32) -> f32 {
    wrap_bob_phase_for_rest(phase) - HPL2_BOB_REST_PHASE
}

fn smoothing_factor(speed: f32, dt: f32) -> f32 {
    1.0 - (-speed.max(0.0) * dt).exp()
}

#[cfg(test)]
mod tests {
    use super::{hpl2_bob_reached_rest, hpl2_bob_step_crossed, move_scalar_toward_slowdown};

    #[test]
    fn scalar_head_offset_movement_uses_hpl2_slowdown() {
        assert_eq!(
            move_scalar_toward_slowdown(1.45, 1.58, 1.6, 0.05, 0.1),
            1.58
        );
        let slowed = move_scalar_toward_slowdown(1.55, 1.58, 1.6, 0.05, 0.01);
        assert!(slowed > 1.55 && slowed < 1.58);
    }

    #[test]
    fn bob_rest_detection_catches_crossing_from_both_directions() {
        let rest = std::f32::consts::FRAC_PI_2;

        assert!(hpl2_bob_reached_rest(rest - 0.1, rest + 0.1));
        assert!(hpl2_bob_reached_rest(rest + 0.1, rest - 0.1));
        assert!(!hpl2_bob_reached_rest(rest - 0.3, rest - 0.1));
    }

    #[test]
    fn bob_step_detection_handles_exact_and_large_phase_crossings() {
        let pi = std::f32::consts::PI;

        assert!(hpl2_bob_step_crossed(pi - 0.1, pi, true));
        assert!(hpl2_bob_step_crossed(pi * 0.9, pi * 2.1, true));
        assert!(!hpl2_bob_step_crossed(pi * 1.1, pi * 1.9, true));
        assert!(!hpl2_bob_step_crossed(pi - 0.1, pi + 0.1, false));
    }
}
