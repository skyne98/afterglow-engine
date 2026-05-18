use avian3d::prelude::{Collider, SpatialQuery, SpatialQueryFilter};
use bevy::prelude::*;

use super::{
    FirstPersonControllerConfig, FirstPersonMotorState, FirstPersonStepRayTrace,
    FirstPersonStepRejectReason, FirstPersonStepTrace,
    util::{flat, shape_fits},
};

pub struct StepAttempt<'a, 'w, 's> {
    pub entity: Entity,
    pub config: &'a FirstPersonControllerConfig,
    pub state: &'a mut FirstPersonMotorState,
    pub transform: &'a mut Transform,
    pub collider: &'a Collider,
    pub spatial_query: &'a SpatialQuery<'w, 's>,
    pub desired_delta: Vec3,
    pub dt: f32,
    pub record_trace: bool,
}

pub fn apply_step_attempt(attempt: StepAttempt) -> FirstPersonStepTrace {
    let StepAttempt {
        entity,
        config,
        state,
        transform,
        collider,
        spatial_query,
        desired_delta,
        dt,
        record_trace,
    } = attempt;
    let was_climbing = state.climbing;
    state.climbing = false;
    if state.step_check_timer > 0.0 {
        return FirstPersonStepTrace::skipped(FirstPersonStepRejectReason::RateLimited);
    }

    let desired_horizontal = flat(desired_delta);
    let Some(move_dir) = desired_horizontal.try_normalize() else {
        return FirstPersonStepTrace::skipped(FirstPersonStepRejectReason::NoHorizontalDelta);
    };

    let firmly_grounded = state.ground_contact_ticks > config.ground_sticky_ticks.saturating_sub(4);
    let max_step = if firmly_grounded || was_climbing {
        config.max_step_height
    } else {
        config.max_step_height_in_air
    };

    let filter = SpatialQueryFilter::from_excluded_entities([entity]);
    let radius = config.body_radius;
    let forward_len = desired_horizontal.length().max(0.05);
    let half_height = config.height(state.stance) * 0.5;

    let num_rays = if config.accurate_climbing { 3 } else { 1 };
    let mut trace = if record_trace {
        FirstPersonStepTrace::running(num_rays, forward_len, max_step)
    } else {
        FirstPersonStepTrace::skipped(FirstPersonStepRejectReason::NotRun)
    };
    let mut rays: [Vec3; 3] = [move_dir; 3];
    if config.accurate_climbing {
        let right_dir = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4) * move_dir;
        let left_dir = Quat::from_rotation_y(-std::f32::consts::FRAC_PI_4) * move_dir;
        rays[1] = right_dir;
        rays[2] = left_dir;
    }

    for i in 0..num_rays {
        let forward_offset = if i == 0 {
            move_dir * (radius + forward_len)
        } else {
            rays[i] * radius + move_dir * forward_len
        };

        let ray_start = transform.translation + forward_offset;
        let ray_end = ray_start - Vec3::Y * half_height;

        let ray_dir = ray_end - ray_start;
        let ray_distance = ray_dir.length();
        if ray_distance <= f32::EPSILON {
            continue;
        }
        let ray_normalized = ray_dir / ray_distance;

        let Ok(dir) = Dir3::new(ray_normalized) else {
            continue;
        };
        let Some(hit) = spatial_query.cast_ray(ray_start, dir, ray_distance, true, &filter) else {
            if record_trace {
                trace.rays.push(FirstPersonStepRayTrace {
                    index: i,
                    start: ray_start,
                    end: ray_end,
                    hit: false,
                    hit_distance: 0.0,
                    step_height: 0.0,
                    fit_position: Vec3::ZERO,
                    reject_reason: FirstPersonStepRejectReason::NoRayHit,
                });
            }
            continue;
        };

        let step_height = half_height - hit.distance;
        let too_low = if was_climbing {
            step_height <= 0.001
        } else {
            step_height <= config.min_step_height
        };
        let too_high = step_height > max_step;
        if too_low || too_high {
            let reject_reason = if too_low {
                FirstPersonStepRejectReason::TooLow
            } else {
                FirstPersonStepRejectReason::TooHigh
            };
            if record_trace {
                trace.reject_reason = reject_reason;
                trace.rays.push(FirstPersonStepRayTrace {
                    index: i,
                    start: ray_start,
                    end: ray_end,
                    hit: true,
                    hit_distance: hit.distance,
                    step_height,
                    fit_position: Vec3::ZERO,
                    reject_reason,
                });
            }
            continue;
        }

        let step_pos = transform.translation
            + Vec3::Y * (step_height + config.step_climb_height_add)
            + move_dir * forward_len * config.climb_forward_mul;

        if !shape_fits(
            entity,
            collider,
            step_pos,
            transform.rotation,
            spatial_query,
        ) {
            if record_trace {
                trace.reject_reason = FirstPersonStepRejectReason::ShapeBlocked;
                trace.rays.push(FirstPersonStepRayTrace {
                    index: i,
                    start: ray_start,
                    end: ray_end,
                    hit: true,
                    hit_distance: hit.distance,
                    step_height,
                    fit_position: step_pos,
                    reject_reason: FirstPersonStepRejectReason::ShapeBlocked,
                });
            }
            continue;
        }

        let climb_remaining = step_height + config.step_climb_height_add;
        let lift = (config.step_climb_speed * dt).min(climb_remaining);
        if lift <= f32::EPSILON {
            continue;
        }
        let start_position = transform.translation;
        let lift_fraction = (lift / climb_remaining).clamp(0.0, 1.0);
        let mut climb_position = start_position;
        climb_position.y += lift;
        climb_position += flat(step_pos - start_position) * lift_fraction;
        if shape_fits(
            entity,
            collider,
            climb_position,
            transform.rotation,
            spatial_query,
        ) {
            transform.translation = climb_position;
        } else {
            transform.translation.y += lift;
        }
        state.velocity.y = 0.0;
        state.climbing = true;
        if record_trace {
            trace.rays.push(FirstPersonStepRayTrace {
                index: i,
                start: ray_start,
                end: ray_end,
                hit: true,
                hit_distance: hit.distance,
                step_height,
                fit_position: step_pos,
                reject_reason: FirstPersonStepRejectReason::Accepted,
            });
            trace.accept(lift);
        }
        break;
    }

    state.step_check_timer = config.step_check_interval;
    trace
}
