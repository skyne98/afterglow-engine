use bevy::prelude::*;
#[allow(unused_imports)]
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PlayerBox {
    pub owner: String,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct KinematicBox {
    pub id: u32,
    pub initial_pos: Vec3,
}

/// Marks a box as roped to a player. This is the replicated state — when
/// present, a local system creates a [`DistanceJoint`] between the box and
/// the player entity matching `player_owner`.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RopedTo {
    pub player_owner: String,
}

/// Marks a locally-spawned joint entity so it can be cleaned up when
/// [`RopedTo`] is removed.
#[derive(Component)]
pub struct RopeJoint;

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
