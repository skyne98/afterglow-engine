use bevy::prelude::*;

use super::protocol::ARENA_HALF;

#[derive(Component)]
pub struct SpawnMarker {
    pub name: String,
}

pub fn arena_extents() -> f32 {
    ARENA_HALF
}
