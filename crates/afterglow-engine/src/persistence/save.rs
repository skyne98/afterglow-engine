use super::*;

pub const LOADED_CELL_SAVE_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LoadedCellSave {
    pub format_version: u32,
    pub chunks: Vec<ChunkPersistentDelta>,
}

impl LoadedCellSave {
    pub fn new(chunks: Vec<ChunkPersistentDelta>) -> Self {
        Self {
            format_version: LOADED_CELL_SAVE_FORMAT_VERSION,
            chunks,
        }
    }

    pub fn to_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

pub fn save_loaded_chunks(
    world: &mut World,
    chunks: impl IntoIterator<Item = ChunkId>,
) -> Result<LoadedCellSave, PersistenceError> {
    let mut deltas = capture_chunk_deltas(world, chunks)?;
    if let Some(stored) = world.get_resource::<PersistentWorldDeltas>() {
        for delta in &mut deltas {
            if let Some(stored_delta) = stored.get(delta.chunk) {
                merge_deleted(delta, &stored_delta.deleted);
            }
        }
    }
    Ok(LoadedCellSave::new(deltas))
}

pub fn load_saved_chunks(
    world: &mut World,
    save: &LoadedCellSave,
) -> Result<ChunkDeltaApplyReport, PersistenceError> {
    if save.format_version != LOADED_CELL_SAVE_FORMAT_VERSION {
        return Err(PersistenceError::UnsupportedSaveVersion {
            version: save.format_version,
        });
    }
    let report = apply_chunk_deltas(world, &save.chunks)?;
    retain_loaded_tombstones(world, &save.chunks);
    Ok(report)
}

pub fn delete_persistent_entity(
    world: &mut World,
    entity: StableEntityId,
) -> Result<bool, PersistenceError> {
    if !entity.is_valid() {
        return Err(PersistenceError::InvalidEntityId);
    }
    maintain_stable_entity_registry(world);

    let Some(world_entity) = world.resource::<StableEntityRegistry>().entity(entity) else {
        return Ok(false);
    };
    if world.get::<Persistent>(world_entity).is_none()
        || world.get::<RuntimeOnly>(world_entity).is_some()
    {
        return Ok(false);
    }
    let chunk = world
        .get::<ChunkMembership>(world_entity)
        .filter(|membership| membership.chunk.is_valid())
        .map(|membership| membership.chunk)
        .ok_or(PersistenceError::InvalidChunkId)?;

    if !world.contains_resource::<PersistentWorldDeltas>() {
        world.insert_resource(PersistentWorldDeltas::default());
    }
    world
        .resource_mut::<PersistentWorldDeltas>()
        .record_deleted(chunk, entity)?;
    let despawned = world.despawn(world_entity);
    if despawned {
        maintain_stable_entity_registry(world);
    }
    Ok(despawned)
}

fn retain_loaded_tombstones(world: &mut World, deltas: &[ChunkPersistentDelta]) {
    if !world.contains_resource::<PersistentWorldDeltas>() {
        world.insert_resource(PersistentWorldDeltas::default());
    }
    let mut store = world.resource_mut::<PersistentWorldDeltas>();
    for delta in deltas {
        for entity in &delta.deleted {
            store
                .record_deleted(delta.chunk, *entity)
                .expect("loaded save was already validated before apply");
        }
    }
}

fn merge_deleted(delta: &mut ChunkPersistentDelta, deleted: &[StableEntityId]) {
    for entity in deleted {
        if !delta.deleted.contains(entity) {
            delta.deleted.push(*entity);
        }
        delta.entities.retain(|entry| entry.entity != *entity);
    }
    delta.deleted.sort();
}
