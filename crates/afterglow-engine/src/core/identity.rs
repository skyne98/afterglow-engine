use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(
    Component,
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    Reflect,
    Serialize,
    Deserialize,
)]
#[reflect(Component, Serialize, Deserialize)]
pub struct StableEntityId(pub u128);

impl StableEntityId {
    pub const INVALID: Self = Self(0);

    pub const fn new(raw: u128) -> Self {
        Self(raw)
    }

    pub const fn from_raw(raw: u128) -> Self {
        Self(raw)
    }

    pub const fn as_raw(self) -> u128 {
        self.0
    }

    pub const fn is_valid(self) -> bool {
        self.0 != 0
    }
}

#[derive(Component, Clone, Copy, Debug, Default, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub struct RuntimeOnly;

#[derive(Resource, Debug, Reflect)]
pub struct StableIdAllocator {
    next: u128,
}

impl Default for StableIdAllocator {
    fn default() -> Self {
        Self { next: 1 }
    }
}

impl StableIdAllocator {
    pub fn allocate(&mut self) -> StableEntityId {
        let id = StableEntityId(self.next);
        self.next = self.next.saturating_add(1);
        id
    }

    pub fn allocate_excluding(&mut self, reserved: &HashSet<StableEntityId>) -> StableEntityId {
        loop {
            let id = self.allocate();
            if id.is_valid() && !reserved.contains(&id) {
                return id;
            }
        }
    }
}
