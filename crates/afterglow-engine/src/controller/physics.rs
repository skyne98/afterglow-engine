use avian3d::{
    character_controller::{move_and_slide::DepenetrationConfig, prelude::MoveAndSlide},
    prelude::{Collider, SpatialQuery, SpatialQueryFilter},
    spatial_query::ShapeCastConfig,
};
use bevy::prelude::*;

use crate::physics::PhysicsCollider;

use super::{
    ControllerStance, FirstPersonControllerConfig, FirstPersonMotorState, is_walkable_normal,
    source_move::source_try_player_move,
    util::{flat, shape_fits},
};

const HPL2_STAND_FIT_Y_OFFSET: f32 = 0.001;
const HPL2_STAND_FIT_SIDE_OFFSET: f32 = 0.01;

pub struct CharacterMove<'a, 'w, 's> {
    pub entity: Entity,
    pub config: &'a FirstPersonControllerConfig,
    pub state: &'a mut FirstPersonMotorState,
    pub transform: &'a mut Transform,
    pub collider: &'a Collider,
    pub move_and_slide: &'a MoveAndSlide<'w, 's>,
    pub spatial_query: &'a SpatialQuery<'w, 's>,
    pub delta: Vec3,
}

pub fn controller_collider(
    config: &FirstPersonControllerConfig,
    stance: ControllerStance,
) -> Collider {
    Collider::cylinder(config.body_radius, config.height(stance))
}

pub fn sync_body_stance(
    entity: Entity,
    config: &FirstPersonControllerConfig,
    state: &mut FirstPersonMotorState,
    transform: &mut Transform,
    spatial_query: &SpatialQuery,
) -> Option<(Collider, PhysicsCollider)> {
    if state.desired_stance == state.stance {
        return None;
    }
    let target = state.desired_stance;
    if target == ControllerStance::Standing {
        let Some(offset) = standing_fit_offset(entity, config, state, transform, spatial_query)
        else {
            state.desired_stance = state.stance;
            return None;
        };
        transform.translation += offset;
    }
    let (new_collider, new_authored) = apply_stance(config, target, state, transform);
    Some((new_collider, new_authored))
}

pub fn apply_horizontal_move(movement: CharacterMove) -> Vec3 {
    let CharacterMove {
        entity,
        config,
        state,
        transform,
        collider,
        move_and_slide: _,
        spatial_query,
        delta,
        ..
    } = movement;
    if delta.length_squared() <= f32::EPSILON {
        return Vec3::ZERO;
    }

    let start = transform.translation;
    let desired_horizontal = flat(delta);
    let actual_delta = source_try_player_move(
        entity,
        config,
        collider,
        transform,
        spatial_query,
        desired_horizontal,
    );
    let blocked_by_step = state.grounded
        && loses_forward_progress(actual_delta, desired_horizontal)
        && is_climbable_step_ahead(entity, config, state, transform, spatial_query, delta);
    let actual_delta = if blocked_by_step {
        let blocked_position = transform.translation;
        transform.translation = start;
        if let Some(step_delta) = step_up_horizontal_move(
            entity,
            config,
            collider,
            transform,
            spatial_query,
            desired_horizontal,
        ) {
            state.climbing = true;
            state.velocity.y = 0.0;
            step_delta
        } else {
            transform.translation = blocked_position;
            actual_delta
        }
    } else {
        actual_delta
    };
    let pushback = actual_delta - flat(delta);
    if !state.grounded && pushback.length_squared() > f32::EPSILON {
        reflect_air_move_speed(state, pushback);
    }
    pushback
}

fn is_climbable_step_ahead(
    entity: Entity,
    config: &FirstPersonControllerConfig,
    state: &FirstPersonMotorState,
    transform: &Transform,
    spatial_query: &SpatialQuery,
    delta: Vec3,
) -> bool {
    let desired_horizontal = flat(delta);
    let Some(move_dir) = desired_horizontal.try_normalize() else {
        return false;
    };
    let half_height = config.height(state.stance) * 0.5;
    let ray_start = transform.translation
        + move_dir * (config.body_radius + desired_horizontal.length().max(0.05));
    let ray_distance = half_height;
    let filter = SpatialQueryFilter::from_excluded_entities([entity]);
    let Some(hit) = spatial_query.cast_ray(ray_start, Dir3::NEG_Y, ray_distance, true, &filter)
    else {
        return false;
    };
    let step_height = half_height - hit.distance;
    if !is_step_height_allowed(step_height, config) {
        return false;
    }
    true
}

fn step_up_horizontal_move(
    entity: Entity,
    config: &FirstPersonControllerConfig,
    collider: &Collider,
    transform: &mut Transform,
    spatial_query: &SpatialQuery,
    desired_horizontal: Vec3,
) -> Option<Vec3> {
    let start = transform.translation;
    let desired_len = desired_horizontal.length();
    let move_dir = desired_horizontal.try_normalize()?;
    let lift = config.max_step_height + config.step_climb_height_add;
    let lifted_start = start + Vec3::Y * lift;
    if !shape_fits(
        entity,
        collider,
        lifted_start,
        transform.rotation,
        spatial_query,
    ) {
        return None;
    }

    transform.translation = lifted_start;
    let raised_delta = source_try_player_move(
        entity,
        config,
        collider,
        transform,
        spatial_query,
        desired_horizontal,
    );
    if flat(raised_delta).dot(move_dir) < desired_len * 0.5 {
        transform.translation = start;
        return None;
    }

    let raised_position = transform.translation;
    let landing = step_down_landing(
        entity,
        config,
        collider,
        raised_position,
        transform.rotation,
        start.y,
        spatial_query,
    )?;
    if !shape_fits(entity, collider, landing, transform.rotation, spatial_query) {
        transform.translation = start;
        return None;
    }

    transform.translation = landing;
    Some(transform.translation - start)
}

fn step_down_landing(
    entity: Entity,
    config: &FirstPersonControllerConfig,
    collider: &Collider,
    raised_position: Vec3,
    rotation: Quat,
    start_y: f32,
    spatial_query: &SpatialQuery,
) -> Option<Vec3> {
    let filter = SpatialQueryFilter::from_excluded_entities([entity]);
    let cast_config = ShapeCastConfig {
        ignore_origin_penetration: true,
        ..ShapeCastConfig::from_max_distance(
            config.max_step_height + config.step_climb_height_add + config.ground_probe_distance,
        )
    };
    let mut hits = spatial_query.shape_hits(
        collider,
        raised_position,
        rotation,
        Dir3::NEG_Y,
        8,
        &cast_config,
        &filter,
    );
    hits.sort_by(|a, b| a.distance.total_cmp(&b.distance));
    hits.into_iter().find_map(|hit| {
        if !is_walkable_normal(hit.normal1, config) {
            return None;
        }
        let contact_center = raised_position - Vec3::Y * hit.distance;
        let step_height = contact_center.y - start_y;
        if !is_step_height_allowed(step_height, config) {
            return None;
        }
        Some(contact_center + Vec3::Y * config.step_climb_height_add)
    })
}

fn loses_forward_progress(actual_delta: Vec3, desired_delta: Vec3) -> bool {
    let Some(move_dir) = desired_delta.try_normalize() else {
        return false;
    };
    flat(actual_delta).dot(move_dir) < desired_delta.length() * 0.5
}

pub fn apply_vertical_force_collision(movement: CharacterMove) -> Vec3 {
    let CharacterMove {
        entity,
        config,
        state,
        transform,
        collider,
        move_and_slide,
        delta,
        ..
    } = movement;
    if delta.y.abs() <= f32::EPSILON {
        return Vec3::ZERO;
    }

    let old_position = transform.translation;
    transform.translation.y += delta.y;
    let pushback = collision_pushback(entity, config, collider, transform, move_and_slide);
    let applied_pushback = Vec3::Y * pushback.y;
    if applied_pushback.length_squared() <= f32::EPSILON {
        super::update_ground_contact(false, Vec3::Y, config.ground_sticky_ticks, state);
        return Vec3::ZERO;
    }

    // HPL2 resolves horizontal movement and vertical forces in separate phases.
    // Avian's depenetration can include lateral components even for a pure-Y
    // probe, so the vertical phase only applies the vertical correction.
    transform.translation = old_position + Vec3::Y * delta.y + applied_pushback;
    let normal = pushback.normalize_or_zero();
    if is_walkable_normal(normal, config) && state.velocity.y <= 0.0 {
        super::update_ground_contact(
            true,
            ground_normal_from_pushback(normal),
            config.ground_sticky_ticks,
            state,
        );
    } else {
        super::update_ground_contact(false, Vec3::Y, config.ground_sticky_ticks, state);
    }

    let clip_normal = no_slide_normal(normal, config);
    let y_velocity = Vec3::Y * state.velocity.y;
    let new_velocity = y_velocity - clip_normal * clip_normal.dot(y_velocity);
    state.velocity.y = new_velocity.y;
    applied_pushback
}

pub fn update_step_climbing(
    config: &FirstPersonControllerConfig,
    state: &mut FirstPersonMotorState,
    dt: f32,
) {
    if state.climbing {
        state.step_check_timer = 0.0;
        state.grounded = true;
        state.ground_contact_ticks = config.ground_sticky_ticks;
    } else {
        state.step_check_timer = (state.step_check_timer - dt).max(0.0);
    }
}

pub fn probe_ground_normal(
    entity: Entity,
    config: &FirstPersonControllerConfig,
    state: &mut FirstPersonMotorState,
    transform: &Transform,
    spatial_query: &SpatialQuery,
) {
    if !state.grounded || state.climbing {
        return;
    }
    let half_height = config.height(state.stance) * 0.5;
    let origin = transform.translation - Vec3::Y * half_height + Vec3::Y * 0.001;
    let max_distance = (config.body_radius * 2.0001).max(config.ground_probe_distance);
    let filter = SpatialQueryFilter::from_excluded_entities([entity]);
    let Some(hit) = spatial_query.cast_ray(origin, Dir3::NEG_Y, max_distance, false, &filter)
    else {
        return;
    };
    if hit.normal.y > 0.2 {
        state.ground_normal = hit.normal.normalize_or(Vec3::Y);
    } else {
        state.ground_normal = Vec3::Y;
    }
}

fn apply_stance(
    config: &FirstPersonControllerConfig,
    stance: ControllerStance,
    state: &mut FirstPersonMotorState,
    transform: &mut Transform,
) -> (Collider, PhysicsCollider) {
    let delta = feet_stable_center_delta(config, state.stance, stance);
    transform.translation.y += delta;
    state.stance = stance;
    (
        controller_collider(config, stance),
        PhysicsCollider::cylinder(config.body_radius, config.height(stance)),
    )
}

fn body_fits(
    entity: Entity,
    config: &FirstPersonControllerConfig,
    stance: ControllerStance,
    position: Vec3,
    spatial_query: &SpatialQuery,
) -> bool {
    let collider = controller_collider(config, stance);
    shape_fits(entity, &collider, position, Quat::IDENTITY, spatial_query)
}

fn standing_fit_offset(
    entity: Entity,
    config: &FirstPersonControllerConfig,
    state: &FirstPersonMotorState,
    transform: &Transform,
    spatial_query: &SpatialQuery,
) -> Option<Vec3> {
    // HPL2 tests standing from feet position before switching active size.
    let feet = feet_position(config, state.stance, transform.translation);
    hpl2_stand_fit_offsets().into_iter().find(|offset| {
        let candidate_feet = feet + *offset;
        let standing_center = center_from_feet(config, ControllerStance::Standing, candidate_feet);
        body_fits(
            entity,
            config,
            ControllerStance::Standing,
            standing_center,
            spatial_query,
        )
    })
}

fn hpl2_stand_fit_offsets() -> [Vec3; 5] {
    [
        Vec3::new(0.0, HPL2_STAND_FIT_Y_OFFSET, 0.0),
        Vec3::new(HPL2_STAND_FIT_SIDE_OFFSET, HPL2_STAND_FIT_Y_OFFSET, 0.0),
        Vec3::new(-HPL2_STAND_FIT_SIDE_OFFSET, HPL2_STAND_FIT_Y_OFFSET, 0.0),
        Vec3::new(0.0, HPL2_STAND_FIT_Y_OFFSET, HPL2_STAND_FIT_SIDE_OFFSET),
        Vec3::new(0.0, HPL2_STAND_FIT_Y_OFFSET, -HPL2_STAND_FIT_SIDE_OFFSET),
    ]
}

fn feet_position(
    config: &FirstPersonControllerConfig,
    stance: ControllerStance,
    center: Vec3,
) -> Vec3 {
    center - Vec3::Y * config.height(stance) * 0.5
}

fn center_from_feet(
    config: &FirstPersonControllerConfig,
    stance: ControllerStance,
    feet: Vec3,
) -> Vec3 {
    feet + Vec3::Y * config.height(stance) * 0.5
}

pub fn feet_stable_center_delta(
    config: &FirstPersonControllerConfig,
    from: ControllerStance,
    to: ControllerStance,
) -> f32 {
    (config.height(to) - config.height(from)) * 0.5
}

pub fn is_step_height_allowed(step_height: f32, config: &FirstPersonControllerConfig) -> bool {
    step_height >= config.min_step_height && step_height <= config.max_step_height
}

fn collision_pushback(
    entity: Entity,
    config: &FirstPersonControllerConfig,
    collider: &Collider,
    transform: &Transform,
    move_and_slide: &MoveAndSlide,
) -> Vec3 {
    let filter = SpatialQueryFilter::from_excluded_entities([entity]);
    move_and_slide.depenetrate(
        collider,
        transform.translation,
        transform.rotation,
        &DepenetrationConfig {
            depenetration_iterations: config.depenetration_iterations,
            skin_width: config.skin_width,
            ..default()
        },
        &filter,
    )
}

fn reflect_air_move_speed(state: &mut FirstPersonMotorState, pushback: Vec3) {
    let normal = pushback.normalize_or_zero();
    if normal == Vec3::ZERO {
        return;
    }
    let rotation = Quat::from_rotation_y(state.yaw);
    let forward = rotation * Vec3::NEG_Z;
    let right = rotation * Vec3::X;
    let forward_velocity = forward * state.forward_speed;
    let side_velocity = right * state.side_speed;
    state.forward_speed = reflected_axis_speed(state.forward_speed, forward_velocity, normal);
    state.side_speed = reflected_axis_speed(state.side_speed, side_velocity, normal);
}

fn reflected_axis_speed(axis_speed: f32, velocity: Vec3, normal: Vec3) -> f32 {
    if axis_speed == 0.0 {
        return 0.0;
    }
    let projected = velocity - normal * normal.dot(velocity);
    let sign = axis_speed.signum();
    let len = projected.length();
    if projected.dot(velocity) >= 0.0 {
        sign * len.min(axis_speed.abs())
    } else {
        0.0
    }
}

fn ground_normal_from_pushback(normal: Vec3) -> Vec3 {
    if normal.y > 0.0 {
        normal.normalize_or(Vec3::Y)
    } else {
        Vec3::Y
    }
}

fn no_slide_normal(normal: Vec3, config: &FirstPersonControllerConfig) -> Vec3 {
    if is_walkable_normal(normal, config) {
        Vec3::Y
    } else {
        normal.normalize_or(Vec3::Y)
    }
}
