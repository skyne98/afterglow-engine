use avian3d::prelude::{Collider, ShapeCastConfig, ShapeHitData, SpatialQuery, SpatialQueryFilter};
use bevy::prelude::*;

use super::{FirstPersonControllerConfig, util::flat, walkable_floor_dot};

const SOURCE_NUM_BUMPS: usize = 4;
const SOURCE_MAX_CLIP_PLANES: usize = 5;
const SOURCE_MOVE_EPSILON: f32 = 0.00001;
const SOURCE_GROUND_CONTACT_LIFT: f32 = 0.01;

pub(crate) fn source_try_player_move(
    entity: Entity,
    config: &FirstPersonControllerConfig,
    collider: &Collider,
    transform: &mut Transform,
    spatial_query: &SpatialQuery,
    delta: Vec3,
) -> Vec3 {
    let start = transform.translation;
    let mut velocity = flat(delta);
    let primal_velocity = velocity;
    let mut original_velocity = velocity;
    let mut time_left = 1.0;
    let mut planes = Vec::with_capacity(SOURCE_MAX_CLIP_PLANES);
    let filter = SpatialQueryFilter::from_excluded_entities([entity]);

    for _ in 0..SOURCE_NUM_BUMPS {
        if velocity.length_squared() <= SOURCE_MOVE_EPSILON * SOURCE_MOVE_EPSILON {
            break;
        }
        let frame_delta = flat(velocity * time_left);
        let distance = frame_delta.length();
        if distance <= SOURCE_MOVE_EPSILON {
            break;
        }
        let move_dir = frame_delta / distance;
        let Some(hit) = first_blocking_shape_hit(
            collider,
            transform.translation + Vec3::Y * SOURCE_GROUND_CONTACT_LIFT,
            transform.rotation,
            move_dir,
            distance,
            walkable_floor_dot(config),
            spatial_query,
            &filter,
        ) else {
            transform.translation += frame_delta;
            break;
        };

        let hit_distance = hit.distance.clamp(0.0, distance);
        let hit_fraction = (hit_distance / distance).clamp(0.0, 1.0);
        if hit_distance > SOURCE_MOVE_EPSILON {
            transform.translation += move_dir * hit_distance;
            original_velocity = velocity;
            planes.clear();
        }
        if hit_fraction >= 1.0 - SOURCE_MOVE_EPSILON {
            break;
        }
        time_left -= time_left * hit_fraction;
        if planes.len() >= SOURCE_MAX_CLIP_PLANES {
            break;
        }
        let Some(normal) =
            horizontal_blocking_normal(hit.normal1, frame_delta, walkable_floor_dot(config))
        else {
            break;
        };
        planes.push(normal);
        velocity = clip_velocity_against_planes(original_velocity, &planes, primal_velocity);
        if velocity.length_squared() <= SOURCE_MOVE_EPSILON * SOURCE_MOVE_EPSILON {
            break;
        }
    }
    transform.translation - start
}

fn first_blocking_shape_hit(
    collider: &Collider,
    origin: Vec3,
    rotation: Quat,
    move_dir: Vec3,
    distance: f32,
    walkable_floor_dot: f32,
    spatial_query: &SpatialQuery,
    filter: &SpatialQueryFilter,
) -> Option<ShapeHitData> {
    let Ok(direction) = Dir3::new(move_dir) else {
        return None;
    };
    let cast_config = ShapeCastConfig {
        ignore_origin_penetration: true,
        ..ShapeCastConfig::from_max_distance(distance)
    };
    let mut hits = spatial_query.shape_hits(
        collider,
        origin,
        rotation,
        direction,
        16,
        &cast_config,
        filter,
    );
    hits.sort_by(|a, b| a.distance.total_cmp(&b.distance));
    hits.into_iter()
        .find(|hit| horizontal_blocking_normal(hit.normal1, move_dir, walkable_floor_dot).is_some())
}

fn horizontal_blocking_normal(
    normal: Vec3,
    movement: Vec3,
    walkable_floor_dot: f32,
) -> Option<Vec3> {
    if normal.y >= walkable_floor_dot {
        return None;
    }
    let normal = flat(normal).try_normalize()?;
    (flat(movement).dot(normal) < -SOURCE_MOVE_EPSILON).then_some(normal)
}

pub(crate) fn clip_velocity_against_planes(
    original_velocity: Vec3,
    planes: &[Vec3],
    primal_velocity: Vec3,
) -> Vec3 {
    for (i, plane) in planes.iter().enumerate() {
        let candidate = clip_velocity(original_velocity, *plane);
        if planes
            .iter()
            .enumerate()
            .all(|(j, other)| i == j || candidate.dot(*other) >= 0.0)
        {
            if candidate.dot(primal_velocity) <= 0.0 {
                return Vec3::ZERO;
            }
            return flat(candidate);
        }
    }
    if planes.len() == 2 {
        let crease = planes[0].cross(planes[1]).normalize_or_zero();
        let candidate = crease * crease.dot(original_velocity);
        if candidate.dot(primal_velocity) > 0.0 {
            return flat(candidate);
        }
    }
    Vec3::ZERO
}

pub(crate) fn clip_velocity(input: Vec3, normal: Vec3) -> Vec3 {
    let backoff = input.dot(normal);
    let mut output = input - normal * backoff;
    let adjust = output.dot(normal);
    if adjust < 0.0 {
        output -= normal * adjust;
    }
    flat(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocking_normal_uses_configured_walkable_slope_threshold() {
        let normal_48_degrees = Vec3::new(0.743, 48.0_f32.to_radians().cos(), 0.0);
        let movement = Vec3::NEG_X;

        assert!(horizontal_blocking_normal(normal_48_degrees, movement, 0.7).is_some());
        assert!(
            horizontal_blocking_normal(normal_48_degrees, movement, 50.0_f32.to_radians().cos(),)
                .is_none()
        );
    }
}
