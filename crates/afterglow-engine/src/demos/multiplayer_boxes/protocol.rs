use bevy::prelude::*;
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

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MoveInput {
    pub direction: Vec2,
}

pub const PLAYER_SPEED: f32 = 5.0;
pub const PLAYER_SIZE: f32 = 0.4;
pub const PLAYER_MASS: f32 = 50.0;
pub const KINEMATIC_BOX_SIZE: f32 = 0.5;
pub const ARENA_HALF: f32 = 10.0;
pub const WALL_HEIGHT: f32 = 3.0;
pub const WALL_THICKNESS: f32 = 0.4;
