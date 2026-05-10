use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Vec3f {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3f {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }

    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }

    fn mul(self, value: f32) -> Self {
        Self::new(self.x * value, self.y * value, self.z * value)
    }

    fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    fn distance_squared(self, other: Self) -> f32 {
        let delta = self.sub(other);
        delta.dot(delta)
    }
}

pub fn segment_distance_squared(a: Vec3f, b: Vec3f, point: Vec3f) -> f32 {
    let segment = b.sub(a);
    let length_squared = segment.dot(segment);
    if length_squared == 0.0 {
        return point.distance_squared(a);
    }
    let t = (point.sub(a).dot(segment) / length_squared).clamp(0.0, 1.0);
    point.distance_squared(a.add(segment.mul(t)))
}
