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

    pub const fn as_hash64(self) -> u64 {
        let raw = self.0;
        ((raw >> 64) as u64) ^ raw as u64
    }
}

/// Generic opt-in marker for entities that should receive a stable engine
/// identity automatically. The marker requires a `StableEntityId` component;
/// `assign_auto_stable_entity_ids` fills invalid ids from `StableIdAllocator`.
#[derive(Component, Clone, Copy, Debug, Default, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
#[require(StableEntityId)]
pub struct AutoStableEntityId;

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
    pub fn reserve_at_least(&mut self, minimum_next: u128) {
        self.next = self.next.max(minimum_next);
    }

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

pub fn assign_auto_stable_entity_ids(
    mut allocator: ResMut<StableIdAllocator>,
    mut entities: Query<&mut StableEntityId, With<AutoStableEntityId>>,
) {
    for mut stable_id in &mut entities {
        if !stable_id.is_valid() {
            *stable_id = allocator.allocate();
        }
    }
}
