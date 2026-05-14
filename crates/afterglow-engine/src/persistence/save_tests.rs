use super::*;
use crate::core::AfterglowCorePlugin;
use serde::{Deserialize, Serialize};

const CHUNK: ChunkId = ChunkId::from_raw(7);
const DOOR: StableEntityId = StableEntityId::from_raw(100);
const NPC: StableEntityId = StableEntityId::from_raw(200);

#[derive(Component, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct SavedDoor {
    open: bool,
}

#[derive(Component, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct SavedHealth {
    current: u32,
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AfterglowCorePlugin,
        AfterglowPersistencePlugin,
    ))
    .persist_component_as::<SavedDoor>("test.door.v1")
    .persist_component_as::<SavedHealth>("test.health.v1");
    app
}

fn spawn_cell_baseline(world: &mut World) {
    world.spawn((
        Persistent,
        DOOR,
        ChunkMembership::new(CHUNK),
        SavedDoor { open: false },
    ));
    world.spawn((
        Persistent,
        NPC,
        ChunkMembership::new(CHUNK),
        SavedHealth { current: 100 },
    ));
}

#[test]
fn loaded_cell_save_roundtrips_and_restores_component_changes_and_tombstones() {
    let mut source = app();
    spawn_cell_baseline(source.world_mut());
    maintain_stable_entity_registry(source.world_mut());
    let door = source
        .world()
        .resource::<StableEntityRegistry>()
        .entity(DOOR)
        .unwrap();
    let npc = source
        .world()
        .resource::<StableEntityRegistry>()
        .entity(NPC)
        .unwrap();
    source
        .world_mut()
        .entity_mut(door)
        .insert(SavedDoor { open: true });
    source
        .world_mut()
        .resource_mut::<PersistentWorldDeltas>()
        .record_deleted(CHUNK, NPC)
        .unwrap();
    assert!(source.world_mut().despawn(npc));

    let save = save_loaded_chunks(source.world_mut(), [CHUNK]).unwrap();
    let bytes = save.to_json().unwrap();
    let decoded = LoadedCellSave::from_json(&bytes).unwrap();

    let mut restored = app();
    spawn_cell_baseline(restored.world_mut());
    let report = load_saved_chunks(restored.world_mut(), &decoded).unwrap();
    let registry = restored.world().resource::<StableEntityRegistry>();
    let restored_door = registry.entity(DOOR).unwrap();

    assert_eq!(decoded.format_version, LOADED_CELL_SAVE_FORMAT_VERSION);
    assert_eq!(report.updated, 1);
    assert_eq!(report.despawned, 1);
    assert_eq!(
        restored.world().get::<SavedDoor>(restored_door),
        Some(&SavedDoor { open: true })
    );
    assert!(registry.entity(NPC).is_none());
}

#[test]
fn loaded_cell_save_retains_tombstones_for_later_saves() {
    let mut source = app();
    spawn_cell_baseline(source.world_mut());
    delete_persistent_entity(source.world_mut(), NPC).unwrap();
    let first_save = save_loaded_chunks(source.world_mut(), [CHUNK]).unwrap();

    let mut restored = app();
    spawn_cell_baseline(restored.world_mut());
    load_saved_chunks(restored.world_mut(), &first_save).unwrap();
    let second_save = save_loaded_chunks(restored.world_mut(), [CHUNK]).unwrap();

    assert_eq!(second_save.chunks.len(), 1);
    assert_eq!(second_save.chunks[0].deleted, [NPC]);
    assert!(
        second_save.chunks[0]
            .entities
            .iter()
            .all(|entity| entity.entity != NPC)
    );
}

#[test]
fn delete_persistent_entity_records_tombstone_and_despawns_loaded_entity() {
    let mut app = app();
    spawn_cell_baseline(app.world_mut());
    maintain_stable_entity_registry(app.world_mut());

    let despawned = delete_persistent_entity(app.world_mut(), NPC).unwrap();

    let store = app.world().resource::<PersistentWorldDeltas>();
    let registry = app.world().resource::<StableEntityRegistry>();
    assert!(despawned);
    assert_eq!(store.get(CHUNK).unwrap().deleted, [NPC]);
    assert!(registry.entity(NPC).is_none());
}

#[test]
fn delete_persistent_entity_rejects_invalid_ids_before_mutating_store() {
    let mut app = app();

    let err = delete_persistent_entity(app.world_mut(), StableEntityId::INVALID).unwrap_err();

    assert!(matches!(err, PersistenceError::InvalidEntityId));
    assert!(
        app.world()
            .resource::<PersistentWorldDeltas>()
            .chunks()
            .is_empty()
    );
}

#[test]
fn delete_persistent_entity_rejects_loaded_entity_without_chunk_before_despawn() {
    let mut app = app();
    let entity = app
        .world_mut()
        .spawn((Persistent, NPC, SavedHealth { current: 100 }))
        .id();

    let err = delete_persistent_entity(app.world_mut(), NPC).unwrap_err();

    assert!(matches!(err, PersistenceError::InvalidChunkId));
    assert!(app.world().get_entity(entity).is_ok());
    assert!(
        app.world()
            .resource::<PersistentWorldDeltas>()
            .chunks()
            .is_empty()
    );
}

#[test]
fn loaded_cell_save_rejects_unsupported_versions_before_mutating_world() {
    let mut app = app();
    spawn_cell_baseline(app.world_mut());
    maintain_stable_entity_registry(app.world_mut());
    let door = app
        .world()
        .resource::<StableEntityRegistry>()
        .entity(DOOR)
        .unwrap();
    let save = LoadedCellSave {
        format_version: LOADED_CELL_SAVE_FORMAT_VERSION + 1,
        chunks: vec![ChunkPersistentDelta {
            chunk: CHUNK,
            entities: vec![PersistentEntityDelta {
                entity: DOOR,
                components: vec![PersistentComponentValue {
                    type_name: "test.door.v1".into(),
                    payload: serde_json::to_vec(&SavedDoor { open: true }).unwrap(),
                }],
                removed_components: vec!["test.door.v1".into()],
            }],
            deleted: Vec::new(),
        }],
    };

    let err = load_saved_chunks(app.world_mut(), &save).unwrap_err();

    assert!(matches!(
        err,
        PersistenceError::UnsupportedSaveVersion { version }
            if version == LOADED_CELL_SAVE_FORMAT_VERSION + 1
    ));
    assert_eq!(
        app.world().get::<SavedDoor>(door),
        Some(&SavedDoor { open: false })
    );
}

#[test]
fn save_loaded_chunks_rejects_invalid_chunk_ids() {
    let mut app = app();

    let err = save_loaded_chunks(app.world_mut(), [ChunkId::INVALID]).unwrap_err();

    assert!(matches!(err, PersistenceError::InvalidChunkId));
}

#[test]
fn record_deleted_deduplicates_tombstones_and_removes_stale_entity_delta() {
    let mut store = PersistentWorldDeltas::default();
    store
        .insert(ChunkPersistentDelta {
            chunk: CHUNK,
            entities: vec![PersistentEntityDelta {
                entity: NPC,
                components: Vec::new(),
                removed_components: Vec::new(),
            }],
            deleted: Vec::new(),
        })
        .unwrap_or_default();

    store.record_deleted(CHUNK, NPC).unwrap();
    store.record_deleted(CHUNK, NPC).unwrap();

    let delta = store.get(CHUNK).unwrap();
    assert!(delta.entities.is_empty());
    assert_eq!(delta.deleted, [NPC]);
}
