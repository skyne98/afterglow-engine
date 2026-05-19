use avian3d::prelude::{Collider, SpatialQuery, SpatialQueryFilter};
use bevy::prelude::*;

pub(crate) fn flat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

pub(crate) fn shape_fits(
    entity: Entity,
    collider: &Collider,
    position: Vec3,
    rotation: Quat,
    spatial_query: &SpatialQuery,
) -> bool {
    let filter = SpatialQueryFilter::from_excluded_entities([entity]);
    spatial_query
        .shape_intersections(collider, position, rotation, &filter)
        .is_empty()
}

pub(crate) fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub(crate) fn local_basis(yaw: f32) -> (Vec3, Vec3) {
    let rotation = Quat::from_rotation_y(yaw);
    (rotation * Vec3::NEG_Z, rotation * Vec3::X)
}
