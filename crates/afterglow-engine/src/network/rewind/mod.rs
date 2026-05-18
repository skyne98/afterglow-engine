//! Entity-level rewind system for network-skeleton entities.
//!
//! Whereas `crate::network::rollback` manages opaque byte-array state per
//! authoritative domain, this module tracks individual component histories
//! for entities that participate in server-authoritative replay /
//! reconciliation.  Each such entity is tagged with [`RewindedEntity`] and
//! its tracked component snapshots are stored in [`ComponentHistory`]
//! indexed by tick.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

use crate::core::identity::StableEntityId;

mod runtime;
pub use runtime::{
    RewindComponentRegistration, RewindComponentRegistry, RewindHistoryStore, RewindTick,
    rewind_type_key,
};

// --------------------------------------------------------------------------
// Identity & domain types
// --------------------------------------------------------------------------

/// Compatibility alias for older rewind-facing code.
///
/// `StableEntityId` is the single universal entity ID used by persistence,
/// replication, and server rewind. New code should use `StableEntityId`
/// directly.
pub type RewindId = StableEntityId;

/// Scoping identifier for a rewind domain — the set of entities that are
/// rewound together as a unit (e.g. all skeletons in a particular
/// simulation region).
#[derive(
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
pub struct RewindDomainId(pub u64);

/// Marker component that opts an entity into server-authoritative rewind
/// tracking.
///
/// Entities with this component have their tracked component histories
/// automatically recorded each tick (up to [`RewindHistoryBudget`]).
#[derive(Component, Clone, Copy, Debug, Default, Reflect)]
#[reflect(Component)]
pub struct RewindedEntity {
    /// The domain this entity belongs to.
    pub domain: RewindDomainId,
    /// Per-entity budget override.  `None` = use the global resource default.
    pub budget_override: Option<usize>,
}

/// Component that selects which component types are tracked for a given
/// rewind domain.
///
/// Attach this to a (ghost) entity or store as a resource per domain.
#[derive(Component, Clone, Debug, Reflect)]
#[reflect(Component)]
pub struct RewindComponentDescriptor {
    pub domain: RewindDomainId,
    /// Fully-qualified type names (e.g.
    /// `"afterglow_engine::physics::Transform"`). The actual tracking is
    /// done via a `TypeRegistry` / `TypeId` map at runtime.
    pub tracked_types: Vec<String>,
}

// --------------------------------------------------------------------------
// History storage
// --------------------------------------------------------------------------

/// Global resource capping the number of historical ticks retained per
/// rewind-tracked entity.
#[derive(Resource, Clone, Copy, Debug, Reflect)]
pub struct RewindHistoryBudget {
    /// Maximum entries in each [`ComponentHistory`] ring buffer.
    pub max_ticks: usize,
    /// If true, old entries are silently dropped; if false, insertion past
    /// capacity is an error (panic in debug builds).
    pub drop_on_overflow: bool,
}

impl Default for RewindHistoryBudget {
    fn default() -> Self {
        Self {
            max_ticks: 120,
            drop_on_overflow: true,
        }
    }
}

/// Per-component-type ring buffer of (`tick`, snapshot) pairs for a single
/// entity.
///
/// Each entity can have multiple `ComponentHistory` entries — one per
/// tracked component type.
#[derive(Component, Clone, Debug, Reflect)]
#[reflect(Component)]
pub struct ComponentHistory {
    /// Domain this history belongs to.
    pub domain: RewindDomainId,
    /// Backing storage ordered by tick (ascending).
    entries: VecDeque<HistoryEntry>,
    /// Maximum capacity derived from [`RewindHistoryBudget`] at init time.
    capacity: usize,
}

/// A single timestamped snapshot inside [`ComponentHistory`].
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub tick: u32,
    /// Opaque serialized component data.
    pub snapshot: Vec<u8>,
}

impl ComponentHistory {
    pub fn with_capacity(capacity: usize, domain: RewindDomainId) -> Self {
        Self {
            domain,
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Push a new snapshot.  If the buffer is full the oldest entry is
    /// evicted (when `drop_on_overflow` is true).
    pub fn push(&mut self, tick: u32, snapshot: Vec<u8>, drop_on_overflow: bool) {
        if self.capacity == 0 {
            return;
        }
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.tick == tick) {
            entry.snapshot = snapshot;
            return;
        }
        let mut insert_at = self
            .entries
            .iter()
            .position(|entry| entry.tick > tick)
            .unwrap_or(self.entries.len());
        if self.entries.len() >= self.capacity {
            if drop_on_overflow {
                if insert_at == 0 {
                    return;
                }
                self.entries.pop_front();
                insert_at -= 1;
            } else {
                debug_assert!(
                    false,
                    "ComponentHistory capacity exceeded for domain {:?}",
                    self.domain
                );
                return;
            }
        }
        self.entries
            .insert(insert_at, HistoryEntry { tick, snapshot });
    }

    /// Retrieve the snapshot nearest to (and not exceeding) `tick`.
    pub fn at_or_before(&self, tick: u32) -> Option<&HistoryEntry> {
        self.entries.iter().rev().find(|e| e.tick <= tick)
    }

    /// Retrieve the snapshot exactly at `tick`.
    pub fn at(&self, tick: u32) -> Option<&HistoryEntry> {
        self.entries.iter().find(|e| e.tick == tick)
    }

    /// Clear all entries with tick <= `tick`.
    pub fn prune_up_to(&mut self, tick: u32) {
        while self.entries.front().is_some_and(|e| e.tick <= tick) {
            self.entries.pop_front();
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &HistoryEntry> {
        self.entries.iter()
    }
}

// --------------------------------------------------------------------------
// Checkpoint / change types
// --------------------------------------------------------------------------

/// A point-in-time snapshot of all tracked components for a single entity.
#[derive(Clone, Debug, Reflect, Serialize, Deserialize)]
pub struct RewindCheckpoint {
    pub stable_id: StableEntityId,
    pub tick: u32,
    /// Map from `TypeId` (serialized as a u64 or string) to opaque blob.
    pub components: Vec<CheckpointComponent>,
}

/// A single component value inside a [`RewindCheckpoint`].
#[derive(Clone, Debug, Reflect, Serialize, Deserialize)]
pub struct CheckpointComponent {
    /// Stable type identifier (e.g. `std::any::TypeId` packed as a u64).
    pub type_key: u64,
    pub data: Vec<u8>,
}

/// Delta between two consecutive checkpoints for the same entity.
#[derive(Clone, Debug, Reflect, Serialize, Deserialize)]
pub struct CheckpointDelta {
    pub from_tick: u32,
    pub to_tick: u32,
    pub changed: Vec<CheckpointComponent>,
    pub removed_type_keys: Vec<u64>,
}

/// A single component mutation at a given tick, produced when comparing
/// two snapshots during replay or reconciliation.
#[derive(Clone, Debug, Message, Reflect, Serialize, Deserialize)]
pub struct ComponentChange {
    pub tick: u32,
    pub stable_id: StableEntityId,
    pub type_key: u64,
    pub old_data: Option<Vec<u8>>,
    pub new_data: Vec<u8>,
}

// --------------------------------------------------------------------------
// Plugin
// --------------------------------------------------------------------------

pub struct ServerRewindPlugin;

impl Plugin for ServerRewindPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RewindHistoryBudget>()
            .init_resource::<crate::core::identity::StableIdAllocator>()
            .init_resource::<RewindTick>()
            .init_resource::<RewindComponentRegistry>()
            .init_resource::<RewindHistoryStore>()
            .init_resource::<runtime::ServerRewindInstalled>()
            .add_message::<ComponentChange>()
            .add_systems(
                FixedPostUpdate,
                (
                    runtime::ensure_rewinded_entities_have_stable_ids,
                    runtime::record_rewind_histories,
                )
                    .chain(),
            )
            .register_type::<StableEntityId>()
            .register_type::<RewindTick>()
            .register_type::<RewindedEntity>()
            .register_type::<RewindComponentDescriptor>()
            .register_type::<ComponentHistory>();
    }
}

// --------------------------------------------------------------------------
// App extension trait
// --------------------------------------------------------------------------

/// Extension trait adding rewind-related convenience methods to [`App`].
pub trait RewindAppExt {
    /// Register a component type for rewind tracking on entities in the
    /// given domain.
    ///
    /// The concrete component type `T` must implement [`Reflect`] and
    /// [`Serialize`]+[`Deserialize`].
    fn register_rewind_component<T>(&mut self, domain: RewindDomainId) -> &mut Self
    where
        T: Component
            + Reflect
            + bevy::reflect::GetTypeRegistration
            + Serialize
            + for<'de> Deserialize<'de>
            + TypePath;

    /// Attach [`ServerRewindPlugin`] and its supporting resources.
    fn add_server_rewind(&mut self) -> &mut Self;

    /// Override the global history budget.
    fn set_rewind_budget(&mut self, max_ticks: usize) -> &mut Self;
}

impl RewindAppExt for App {
    fn register_rewind_component<T>(&mut self, domain: RewindDomainId) -> &mut Self
    where
        T: Component
            + Reflect
            + bevy::reflect::GetTypeRegistration
            + Serialize
            + for<'de> Deserialize<'de>
            + TypePath,
    {
        self.world_mut()
            .get_resource_or_insert_with(RewindHistoryBudget::default);
        self.init_resource::<RewindComponentRegistry>();
        self.world_mut()
            .resource_mut::<RewindComponentRegistry>()
            .register::<T>(domain);
        self.register_type::<T>();
        self
    }

    fn add_server_rewind(&mut self) -> &mut Self {
        if !self
            .world()
            .contains_resource::<runtime::ServerRewindInstalled>()
        {
            self.add_plugins(ServerRewindPlugin);
        }
        self
    }

    fn set_rewind_budget(&mut self, max_ticks: usize) -> &mut Self {
        let mut budget = self
            .world_mut()
            .get_resource_or_insert_with(RewindHistoryBudget::default);
        budget.max_ticks = max_ticks;
        self
    }
}

#[cfg(test)]
mod tests;
