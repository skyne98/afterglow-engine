use bevy::prelude::Reflect;
use serde::{Deserialize, Serialize};

pub mod network_e2e;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Player(pub u64);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Chunk(pub i32, pub i32, pub i32);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Entity(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect, Serialize, Deserialize)]
pub struct Vec3i {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl Vec3i {
    pub const ZERO: Self = Self::new(0, 0, 0);

    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    pub fn distance_squared(self, other: Self) -> i64 {
        let dx = i64::from(self.x) - i64::from(other.x);
        let dy = i64::from(self.y) - i64::from(other.y);
        let dz = i64::from(self.z) - i64::from(other.z);
        dx * dx + dy * dy + dz * dz
    }

    pub fn chunk(self) -> Chunk {
        Chunk(
            self.x.div_euclid(32),
            self.y.div_euclid(16),
            self.z.div_euclid(32),
        )
    }
}

pub fn valid_move(current: Vec3i, target: Vec3i) -> bool {
    current.distance_squared(target) <= 160_i64 * 160
}

pub fn in_reach(a: Vec3i, b: Vec3i) -> bool {
    a.distance_squared(b) <= 8_i64 * 8
}

pub fn near(a: Chunk, b: Chunk) -> bool {
    (a.0 - b.0).abs() <= 1 && (a.1 - b.1).abs() <= 1 && (a.2 - b.2).abs() <= 1
}
