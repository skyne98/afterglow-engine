use bevy::{ecs::query::Or, prelude::*};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
pub struct ChunkId(pub u64);

impl ChunkId {
    pub const INVALID: Self = Self(0);

    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn as_raw(self) -> u64 {
        self.0
    }

    pub const fn is_valid(self) -> bool {
        self.0 != 0
    }
}

#[derive(Component, Clone, Copy, Debug, Eq, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub struct ChunkMembership {
    pub chunk: ChunkId,
}

impl ChunkMembership {
    pub const fn new(chunk: ChunkId) -> Self {
        Self { chunk }
    }
}

#[derive(Component, Clone, Copy, Debug, Default, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub struct Persistent;

#[derive(Component, Clone, Copy, Debug, Default, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub struct Replicated;

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
}

#[derive(Resource, Debug, Default, Reflect)]
pub struct StableEntityRegistry {
    stable_to_entity: HashMap<StableEntityId, Entity>,
    entity_to_stable: HashMap<Entity, StableEntityId>,
    chunk_to_entities: HashMap<ChunkId, Vec<Entity>>,
    duplicate_ids: Vec<StableEntityId>,
}

impl StableEntityRegistry {
    pub fn entity(&self, stable_id: StableEntityId) -> Option<Entity> {
        self.stable_to_entity.get(&stable_id).copied()
    }

    pub fn stable_id(&self, entity: Entity) -> Option<StableEntityId> {
        self.entity_to_stable.get(&entity).copied()
    }

    pub fn chunk_entities(&self, chunk: ChunkId) -> &[Entity] {
        self.chunk_to_entities
            .get(&chunk)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn duplicate_ids(&self) -> &[StableEntityId] {
        &self.duplicate_ids
    }

    pub fn len(&self) -> usize {
        self.entity_to_stable.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entity_to_stable.is_empty()
    }

    fn clear(&mut self) {
        self.stable_to_entity.clear();
        self.entity_to_stable.clear();
        self.chunk_to_entities.clear();
        self.duplicate_ids.clear();
    }
}

pub fn maintain_stable_entity_registry(world: &mut World) {
    let missing = {
        let mut query = world.query_filtered::<Entity, (
            Or<(With<Persistent>, With<Replicated>)>,
            Without<RuntimeOnly>,
            Without<StableEntityId>,
        )>();
        query.iter(world).collect::<Vec<_>>()
    };

    for entity in missing {
        let id = world.resource_mut::<StableIdAllocator>().allocate();
        if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
            entity_mut.insert(id);
        }
    }

    let entries = {
        let mut query = world.query::<(Entity, &StableEntityId, Option<&ChunkMembership>)>();
        query
            .iter(world)
            .map(|(entity, stable_id, chunk)| (entity, *stable_id, chunk.copied()))
            .collect::<Vec<_>>()
    };

    let mut registry = world.resource_mut::<StableEntityRegistry>();
    registry.clear();

    for (entity, stable_id, chunk) in entries {
        if !stable_id.is_valid() {
            continue;
        }

        if registry
            .stable_to_entity
            .insert(stable_id, entity)
            .is_some()
        {
            registry.duplicate_ids.push(stable_id);
        }

        registry.entity_to_stable.insert(entity, stable_id);

        if let Some(chunk) = chunk {
            if chunk.chunk.is_valid() {
                registry
                    .chunk_to_entities
                    .entry(chunk.chunk)
                    .or_default()
                    .push(entity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::AfterglowCorePlugin;

    #[test]
    fn assigns_stable_ids_to_persistent_entities() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AfterglowCorePlugin));

        let entity = app.world_mut().spawn(Persistent).id();
        app.update();

        let stable_id = app.world().get::<StableEntityId>(entity).copied();
        assert!(stable_id.is_some_and(StableEntityId::is_valid));

        let registry = app.world().resource::<StableEntityRegistry>();
        assert_eq!(registry.entity(stable_id.unwrap()), Some(entity));
        assert_eq!(registry.stable_id(entity), stable_id);
    }

    #[test]
    fn tracks_chunk_membership() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AfterglowCorePlugin));

        let chunk = ChunkId::from_raw(7);
        let first = app
            .world_mut()
            .spawn((Persistent, ChunkMembership::new(chunk)))
            .id();
        let second = app
            .world_mut()
            .spawn((
                StableEntityId::from_raw(99),
                Replicated,
                ChunkMembership::new(chunk),
            ))
            .id();

        app.update();

        let registry = app.world().resource::<StableEntityRegistry>();
        let entities = registry.chunk_entities(chunk);
        assert!(entities.contains(&first));
        assert!(entities.contains(&second));
    }

    #[test]
    fn ignores_runtime_only_entities_for_auto_assignment() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AfterglowCorePlugin));

        let entity = app.world_mut().spawn((Persistent, RuntimeOnly)).id();
        app.update();

        assert!(app.world().get::<StableEntityId>(entity).is_none());
        assert!(app.world().resource::<StableEntityRegistry>().is_empty());
    }

    #[test]
    fn records_duplicate_stable_ids() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AfterglowCorePlugin));

        let duplicate = StableEntityId::from_raw(42);
        app.world_mut().spawn((duplicate, Persistent));
        app.world_mut().spawn((duplicate, Persistent));
        app.update();

        let registry = app.world().resource::<StableEntityRegistry>();
        assert_eq!(registry.duplicate_ids(), &[duplicate]);
    }

    #[test]
    fn removes_despawned_entities_from_registry() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AfterglowCorePlugin));

        let entity = app.world_mut().spawn(Persistent).id();
        app.update();
        assert_eq!(app.world().resource::<StableEntityRegistry>().len(), 1);

        app.world_mut().despawn(entity);
        app.update();
        assert!(app.world().resource::<StableEntityRegistry>().is_empty());
    }
}
