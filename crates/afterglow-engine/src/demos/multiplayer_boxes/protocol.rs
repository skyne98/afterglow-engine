use bevy::prelude::*;
#[allow(unused_imports)]
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::identity::StableEntityId;

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PlayerBox {
    pub owner: String,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct KinematicBox {
    pub initial_pos: Vec3,
}

/// Entity-backed rope state. Clients may spawn this as a Lightyear
/// `PreSpawned` predicted entity; the server confirms it by spawning a
/// replicated `RopeLink` with the same deterministic hash.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RopeLink {
    pub rope_id: StableEntityId,
    pub player_owner: String,
    pub target: StableEntityId,
}

/// Marks a locally-spawned joint entity so it can be cleaned up when its
/// owning [`RopeLink`] is removed.
#[derive(Component)]
pub struct RopeJoint;

pub fn kinematic_box_hue(id: StableEntityId) -> f32 {
    // Spread sequential runtime IDs around the hue circle instead of clustering
    // `1, 2, 3, ...` into nearly identical reds.
    (id.as_hash64().wrapping_mul(137) % 360) as f32
}

pub const PLAYER_SPEED: f32 = 5.0;
pub const PLAYER_SIZE: f32 = 0.4;
pub const PLAYER_MASS: f32 = 50.0;
pub const KINEMATIC_BOX_SIZE: f32 = 0.5;
pub const ARENA_HALF: f32 = 10.0;
pub const WALL_HEIGHT: f32 = 3.0;
pub const WALL_THICKNESS: f32 = 0.4;

/// Max rope length (distance joint upper limit).
pub const ROPE_MAX_DISTANCE: f32 = 3.0;
/// How close a box must be to be roped.
pub const ROPE_GRAB_RANGE: f32 = 3.0;
/// Joint compliance (inverse stiffness). Higher = softer rope.
pub const ROPE_COMPLIANCE: f32 = 0.001;
