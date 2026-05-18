use crate::{
    core::identity::{ChunkId, StableEntityRegistry, maintain_stable_entity_registry},
    persistence::{PersistentWorldDeltas, apply_chunk_delta, save_loaded_chunks},
};
use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Reflect)]
pub enum ChunkLifecycleState {
    #[default]
    Unloaded,
    Loading,
    Spawned,
    GameplayActive,
    Sleeping,
    Unloading,
}

#[derive(Resource, Clone, Debug, Default, Eq, PartialEq, Reflect)]
pub struct ChunkLifecycle {
    states: BTreeMap<ChunkId, ChunkLifecycleState>,
}

#[derive(Resource, Clone, Debug, Eq, PartialEq, Reflect)]
pub struct ChunkLifecycleConfig {
    pub save_on_unload: bool,
    pub apply_saved_delta_on_spawned: bool,
}

impl Default for ChunkLifecycleConfig {
    fn default() -> Self {
        Self {
            save_on_unload: true,
            apply_saved_delta_on_spawned: true,
        }
    }
}

#[derive(Resource, Clone, Debug, Default, Eq, PartialEq)]
pub struct ChunkLifecycleRequests {
    load: BTreeSet<ChunkId>,
    spawned: BTreeSet<ChunkId>,
    activate: BTreeSet<ChunkId>,
    sleep: BTreeSet<ChunkId>,
    unload: BTreeSet<ChunkId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChunkLifecycleTransition {
    pub chunk: ChunkId,
    pub from: ChunkLifecycleState,
    pub to: ChunkLifecycleState,
}

#[derive(Resource, Clone, Debug, Default, Eq, PartialEq)]
pub struct ChunkLifecycleReport {
    pub transitions: Vec<ChunkLifecycleTransition>,
    pub saved_chunks: Vec<ChunkId>,
    pub applied_saved_chunks: Vec<ChunkId>,
    pub despawned_entities: usize,
    pub errors: Vec<ChunkLifecycleError>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkLifecycleError {
    pub chunk: ChunkId,
    pub message: String,
}

impl ChunkLifecycle {
    pub fn state(&self, chunk: ChunkId) -> ChunkLifecycleState {
        self.states.get(&chunk).copied().unwrap_or_default()
    }

    pub fn chunks(&self) -> &BTreeMap<ChunkId, ChunkLifecycleState> {
        &self.states
    }

    fn set_state(
        &mut self,
        chunk: ChunkId,
        state: ChunkLifecycleState,
    ) -> ChunkLifecycleTransition {
        let from = self.state(chunk);
        if state == ChunkLifecycleState::Unloaded {
            self.states.remove(&chunk);
        } else {
            self.states.insert(chunk, state);
        }
        ChunkLifecycleTransition {
            chunk,
            from,
            to: state,
        }
    }
}

impl ChunkLifecycleRequests {
    pub fn request_load(&mut self, chunk: ChunkId) -> Result<(), ChunkLifecycleRequestError> {
        insert_valid(&mut self.load, chunk)
    }

    pub fn request_spawned(&mut self, chunk: ChunkId) -> Result<(), ChunkLifecycleRequestError> {
        insert_valid(&mut self.spawned, chunk)
    }

    pub fn request_activate(&mut self, chunk: ChunkId) -> Result<(), ChunkLifecycleRequestError> {
        insert_valid(&mut self.activate, chunk)
    }

    pub fn request_sleep(&mut self, chunk: ChunkId) -> Result<(), ChunkLifecycleRequestError> {
        insert_valid(&mut self.sleep, chunk)
    }

    pub fn request_unload(&mut self, chunk: ChunkId) -> Result<(), ChunkLifecycleRequestError> {
        insert_valid(&mut self.unload, chunk)
    }

    fn clear(&mut self) {
        self.load.clear();
        self.spawned.clear();
        self.activate.clear();
        self.sleep.clear();
        self.unload.clear();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ChunkLifecycleRequestError {
    #[error("chunk lifecycle request has invalid chunk id")]
    InvalidChunkId,
}

pub fn process_chunk_lifecycle_requests(world: &mut World) {
    let requests = world.resource::<ChunkLifecycleRequests>().clone();
    world.resource_mut::<ChunkLifecycleRequests>().clear();
    world.insert_resource(ChunkLifecycleReport::default());

    for chunk in &requests.unload {
        unload_chunk(world, *chunk);
    }
    for chunk in requests.load.difference(&requests.unload) {
        if world.resource::<ChunkLifecycle>().state(*chunk) == ChunkLifecycleState::Unloaded {
            transition_if_not(world, *chunk, ChunkLifecycleState::Loading);
        }
    }
    for chunk in requests.spawned.difference(&requests.unload) {
        let state = world.resource::<ChunkLifecycle>().state(*chunk);
        if matches!(
            state,
            ChunkLifecycleState::Unloaded | ChunkLifecycleState::Loading
        ) && apply_saved_delta_if_enabled(world, *chunk)
        {
            transition_if_not(world, *chunk, ChunkLifecycleState::Spawned);
        }
    }
    for chunk in requests.activate.difference(&requests.unload) {
        let state = world.resource::<ChunkLifecycle>().state(*chunk);
        if matches!(
            state,
            ChunkLifecycleState::Spawned | ChunkLifecycleState::Sleeping
        ) {
            transition_if_not(world, *chunk, ChunkLifecycleState::GameplayActive);
        }
    }
    for chunk in requests.sleep.difference(&requests.unload) {
        let state = world.resource::<ChunkLifecycle>().state(*chunk);
        if matches!(
            state,
            ChunkLifecycleState::Spawned | ChunkLifecycleState::GameplayActive
        ) {
            transition_if_not(world, *chunk, ChunkLifecycleState::Sleeping);
        }
    }
}

fn unload_chunk(world: &mut World, chunk: ChunkId) {
    let previous = world.resource::<ChunkLifecycle>().state(chunk);
    if previous == ChunkLifecycleState::Unloaded {
        return;
    }
    transition_if_not(world, chunk, ChunkLifecycleState::Unloading);
    let should_save = matches!(
        previous,
        ChunkLifecycleState::Spawned
            | ChunkLifecycleState::GameplayActive
            | ChunkLifecycleState::Sleeping
    );
    if should_save && world.resource::<ChunkLifecycleConfig>().save_on_unload {
        match save_loaded_chunks(world, [chunk]) {
            Ok(saved) => {
                for delta in saved.chunks {
                    world.resource_mut::<PersistentWorldDeltas>().insert(delta);
                }
                world
                    .resource_mut::<ChunkLifecycleReport>()
                    .saved_chunks
                    .push(chunk);
            }
            Err(err) => {
                push_error(world, chunk, err.to_string());
                transition_if_not(world, chunk, previous);
                return;
            }
        }
    }

    maintain_stable_entity_registry(world);
    let entities = world
        .resource::<StableEntityRegistry>()
        .chunk_entities(chunk)
        .to_vec();
    let mut despawned = 0;
    for entity in entities {
        if world.despawn(entity) {
            despawned += 1;
        }
    }
    if despawned > 0 {
        maintain_stable_entity_registry(world);
    }
    world
        .resource_mut::<ChunkLifecycleReport>()
        .despawned_entities += despawned;
    transition_if_not(world, chunk, ChunkLifecycleState::Unloaded);
}

fn apply_saved_delta_if_enabled(world: &mut World, chunk: ChunkId) -> bool {
    if !world
        .resource::<ChunkLifecycleConfig>()
        .apply_saved_delta_on_spawned
    {
        return true;
    }
    let Some(delta) = world
        .resource::<PersistentWorldDeltas>()
        .get(chunk)
        .cloned()
    else {
        return true;
    };
    match apply_chunk_delta(world, &delta) {
        Ok(_) => {
            world
                .resource_mut::<ChunkLifecycleReport>()
                .applied_saved_chunks
                .push(chunk);
            true
        }
        Err(err) => {
            push_error(world, chunk, err.to_string());
            false
        }
    }
}

fn transition_if_not(world: &mut World, chunk: ChunkId, to: ChunkLifecycleState) {
    let transition = {
        let mut lifecycle = world.resource_mut::<ChunkLifecycle>();
        (lifecycle.state(chunk) != to).then(|| lifecycle.set_state(chunk, to))
    };
    if let Some(transition) = transition {
        world
            .resource_mut::<ChunkLifecycleReport>()
            .transitions
            .push(transition);
    }
}

fn push_error(world: &mut World, chunk: ChunkId, message: String) {
    world
        .resource_mut::<ChunkLifecycleReport>()
        .errors
        .push(ChunkLifecycleError { chunk, message });
}

fn insert_valid(
    chunks: &mut BTreeSet<ChunkId>,
    chunk: ChunkId,
) -> Result<(), ChunkLifecycleRequestError> {
    if !chunk.is_valid() {
        return Err(ChunkLifecycleRequestError::InvalidChunkId);
    }
    chunks.insert(chunk);
    Ok(())
}

#[cfg(test)]
mod tests;
