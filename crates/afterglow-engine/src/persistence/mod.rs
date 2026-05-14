use crate::core::identity::{
    ChunkId, ChunkMembership, Persistent, RuntimeOnly, StableEntityId, StableEntityRegistry,
    maintain_stable_entity_registry,
};
use bevy::prelude::*;
use serde::{Serialize, de::DeserializeOwned};
use std::{
    any::type_name,
    collections::{BTreeMap, BTreeSet},
};

mod apply;
pub use apply::{apply_chunk_delta, apply_chunk_deltas};
mod save;
pub use save::{
    LOADED_CELL_SAVE_FORMAT_VERSION, LoadedCellSave, delete_persistent_entity, load_saved_chunks,
    save_loaded_chunks,
};

#[derive(Resource, Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PersistentWorldDeltas {
    chunks: BTreeMap<ChunkId, ChunkPersistentDelta>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChunkPersistentDelta {
    pub chunk: ChunkId,
    pub entities: Vec<PersistentEntityDelta>,
    pub deleted: Vec<StableEntityId>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PersistentEntityDelta {
    pub entity: StableEntityId,
    pub components: Vec<PersistentComponentValue>,
    pub removed_components: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PersistentComponentValue {
    pub type_name: String,
    pub payload: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ChunkDeltaApplyReport {
    pub updated: usize,
    pub spawned: usize,
    pub despawned: usize,
    pub components_applied: usize,
    pub components_removed: usize,
}

#[derive(Resource, Default)]
pub struct PersistenceRegistry {
    components: BTreeMap<&'static str, PersistentComponentRuntime>,
}

struct PersistentComponentRuntime {
    capture: CapturePersistentComponent,
    deserialize: DeserializePersistentComponent,
    remove: fn(&mut World, Entity) -> bool,
}

type CapturePersistentComponent =
    fn(&World, Entity, &'static str) -> Result<Option<Vec<u8>>, PersistenceError>;
type DeserializePersistentComponent =
    fn(&[u8], &'static str) -> Result<Box<dyn PersistentComponentApply>, PersistenceError>;

trait PersistentComponentApply {
    fn apply(self: Box<Self>, world: &mut World, entity: Entity);
}

struct TypedPersistentComponent<T>(T);

impl<T> PersistentComponentApply for TypedPersistentComponent<T>
where
    T: Component,
{
    fn apply(self: Box<Self>, world: &mut World, entity: Entity) {
        world.entity_mut(entity).insert(self.0);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PersistenceError {
    #[error("component {type_name} is not registered for persistence")]
    UnregisteredComponent { type_name: String },
    #[error("chunk delta has invalid chunk id")]
    InvalidChunkId,
    #[error("chunk delta contains invalid entity id")]
    InvalidEntityId,
    #[error("entity {entity:?} appears in multiple chunk delta records")]
    DuplicateEntityDelta { entity: StableEntityId },
    #[error("entity {entity:?} is both restored and deleted in the same batch")]
    ConflictingEntityDelta { entity: StableEntityId },
    #[error("loaded cell save version {version} is not supported")]
    UnsupportedSaveVersion { version: u32 },
    #[error("failed to serialize component {type_name}: {message}")]
    Serialize {
        type_name: &'static str,
        message: String,
    },
    #[error("failed to deserialize component {type_name}: {message}")]
    Deserialize {
        type_name: &'static str,
        message: String,
    },
}

pub struct AfterglowPersistencePlugin;

impl Plugin for AfterglowPersistencePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PersistentWorldDeltas>()
            .init_resource::<PersistenceRegistry>();
    }
}

pub trait PersistenceAppExt {
    fn persist_component<T>(&mut self) -> &mut Self
    where
        T: Component + Serialize + DeserializeOwned;
    fn persist_component_as<T>(&mut self, stable_name: &'static str) -> &mut Self
    where
        T: Component + Serialize + DeserializeOwned;
}

impl PersistenceAppExt for App {
    fn persist_component<T>(&mut self) -> &mut Self
    where
        T: Component + Serialize + DeserializeOwned,
    {
        self.persist_component_as::<T>(type_name::<T>())
    }

    fn persist_component_as<T>(&mut self, stable_name: &'static str) -> &mut Self
    where
        T: Component + Serialize + DeserializeOwned,
    {
        self.init_resource::<PersistenceRegistry>();
        let mut registry = self.world_mut().resource_mut::<PersistenceRegistry>();
        registry.components.insert(
            stable_name,
            PersistentComponentRuntime {
                capture: capture_component::<T>,
                deserialize: deserialize_component::<T>,
                remove: remove_component::<T>,
            },
        );
        self
    }
}

impl PersistentWorldDeltas {
    pub fn insert(&mut self, delta: ChunkPersistentDelta) -> Option<ChunkPersistentDelta> {
        self.chunks.insert(delta.chunk, delta)
    }

    pub fn get(&self, chunk: ChunkId) -> Option<&ChunkPersistentDelta> {
        self.chunks.get(&chunk)
    }

    pub fn remove(&mut self, chunk: ChunkId) -> Option<ChunkPersistentDelta> {
        self.chunks.remove(&chunk)
    }

    pub fn chunks(&self) -> &BTreeMap<ChunkId, ChunkPersistentDelta> {
        &self.chunks
    }

    pub fn record_deleted(
        &mut self,
        chunk: ChunkId,
        entity: StableEntityId,
    ) -> Result<(), PersistenceError> {
        if !chunk.is_valid() {
            return Err(PersistenceError::InvalidChunkId);
        }
        if !entity.is_valid() {
            return Err(PersistenceError::InvalidEntityId);
        }
        let delta = self
            .chunks
            .entry(chunk)
            .or_insert_with(|| ChunkPersistentDelta {
                chunk,
                entities: Vec::new(),
                deleted: Vec::new(),
            });
        delta.entities.retain(|entry| entry.entity != entity);
        if !delta.deleted.contains(&entity) {
            delta.deleted.push(entity);
            delta.deleted.sort();
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

pub fn capture_chunk_delta(
    world: &mut World,
    chunk: ChunkId,
) -> Result<ChunkPersistentDelta, PersistenceError> {
    let mut deltas = capture_chunk_deltas(world, [chunk])?;
    Ok(deltas.pop().unwrap_or(ChunkPersistentDelta {
        chunk,
        entities: Vec::new(),
        deleted: Vec::new(),
    }))
}

pub fn capture_chunk_deltas(
    world: &mut World,
    chunks: impl IntoIterator<Item = ChunkId>,
) -> Result<Vec<ChunkPersistentDelta>, PersistenceError> {
    maintain_stable_entity_registry(world);
    let requested_chunks = chunks.into_iter().collect::<BTreeSet<_>>();
    if requested_chunks.iter().any(|chunk| !chunk.is_valid()) {
        return Err(PersistenceError::InvalidChunkId);
    }
    let mut deltas = requested_chunks
        .iter()
        .map(|chunk| {
            (
                *chunk,
                ChunkPersistentDelta {
                    chunk: *chunk,
                    entities: Vec::new(),
                    deleted: Vec::new(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let component_names = world.resource::<PersistenceRegistry>().component_names();
    if requested_chunks.is_empty() || component_names.is_empty() {
        return Ok(deltas.into_values().collect());
    }

    let entries = {
        let registry = world.resource::<StableEntityRegistry>();
        requested_chunks
            .iter()
            .flat_map(|chunk| {
                registry
                    .chunk_entities(*chunk)
                    .iter()
                    .copied()
                    .map(|entity| (*chunk, entity))
            })
            .collect::<Vec<_>>()
    };

    for (chunk, entity) in entries {
        if world.get::<Persistent>(entity).is_none() || world.get::<RuntimeOnly>(entity).is_some() {
            continue;
        }
        let Some(stable_id) = world.get::<StableEntityId>(entity).copied() else {
            continue;
        };
        if !stable_id.is_valid() {
            continue;
        }
        let components = world
            .resource::<PersistenceRegistry>()
            .capture_entity(world, entity)?;
        let delta = deltas
            .get_mut(&chunk)
            .expect("requested chunk delta should exist");
        delta.entities.push(PersistentEntityDelta {
            entity: stable_id,
            components,
            removed_components: component_names.clone(),
        });
    }
    for delta in deltas.values_mut() {
        delta.entities.sort_by_key(|entity| entity.entity);
    }

    Ok(deltas.into_values().collect())
}

impl PersistenceRegistry {
    pub fn component_names(&self) -> Vec<String> {
        self.components
            .keys()
            .map(|name| (*name).to_string())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.components.len()
    }

    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }

    fn capture_entity(
        &self,
        world: &World,
        entity: Entity,
    ) -> Result<Vec<PersistentComponentValue>, PersistenceError> {
        self.components
            .iter()
            .filter_map(
                |(type_name, runtime)| match (runtime.capture)(world, entity, type_name) {
                    Ok(Some(payload)) => Some(Ok(PersistentComponentValue {
                        type_name: (*type_name).to_string(),
                        payload,
                    })),
                    Ok(None) => None,
                    Err(err) => Some(Err(err)),
                },
            )
            .collect()
    }
}

fn capture_component<T>(
    world: &World,
    entity: Entity,
    type_name: &'static str,
) -> Result<Option<Vec<u8>>, PersistenceError>
where
    T: Component + Serialize,
{
    let Some(component) = world.get::<T>(entity) else {
        return Ok(None);
    };
    serde_json::to_vec(component)
        .map(Some)
        .map_err(|err| PersistenceError::Serialize {
            type_name,
            message: err.to_string(),
        })
}

fn deserialize_component<T>(
    payload: &[u8],
    type_name: &'static str,
) -> Result<Box<dyn PersistentComponentApply>, PersistenceError>
where
    T: Component + DeserializeOwned,
{
    let component =
        serde_json::from_slice::<T>(payload).map_err(|err| PersistenceError::Deserialize {
            type_name,
            message: err.to_string(),
        })?;
    Ok(Box::new(TypedPersistentComponent(component)))
}

fn remove_component<T>(world: &mut World, entity: Entity) -> bool
where
    T: Component,
{
    let mut entity = world.entity_mut(entity);
    if entity.contains::<T>() {
        entity.remove::<T>();
        true
    } else {
        false
    }
}

#[cfg(test)]
mod edge_tests;
#[cfg(test)]
mod save_tests;
#[cfg(test)]
mod tests;
