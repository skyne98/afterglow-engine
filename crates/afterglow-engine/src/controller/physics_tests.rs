use super::source_move::{clip_velocity, clip_velocity_against_planes};
use bevy::prelude::*;

#[test]
fn source_clip_velocity_slides_along_blocking_plane() {
    let input = Vec3::new(0.03, 0.0, 0.02);
    let wall_normal = Vec3::NEG_X;

    let clipped = clip_velocity(input, wall_normal);

    assert_eq!(clipped.x, 0.0);
    assert_eq!(clipped.z, input.z);
}

#[test]
fn source_clip_velocity_does_not_inject_tangent_axis_motion() {
    let original = Vec3::new(0.034356136, 0.0, -0.000027442382);
    let wall_normal = Vec3::NEG_X;

    let clipped = clip_velocity_against_planes(original, &[wall_normal], original);

    assert_eq!(clipped.x, 0.0);
    assert_eq!(clipped.z, original.z);
}

#[test]
fn source_clip_velocity_stops_when_corner_turns_against_original_motion() {
    let original = Vec3::new(0.03, 0.0, 0.02);
    let planes = [Vec3::NEG_X, Vec3::NEG_Z];

    let clipped = clip_velocity_against_planes(original, &planes, original);

    assert_eq!(clipped, Vec3::ZERO);
}
