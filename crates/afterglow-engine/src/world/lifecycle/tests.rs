use super::*;
use crate::{
    core::{
        AfterglowCorePlugin,
        identity::{
            ChunkMembership, Persistent, RuntimeOnly, StableEntityId, StableEntityRegistry,
        },
    },
    persistence::{
        AfterglowPersistencePlugin, ChunkPersistentDelta, PersistenceAppExt,
        PersistentComponentValue, PersistentEntityDelta, PersistentWorldDeltas,
    },
};
use serde::{Deserialize, Serialize};

const CHUNK: ChunkId = ChunkId::from_raw(7);
const OTHER_CHUNK: ChunkId = ChunkId::from_raw(8);
const DOOR: StableEntityId = StableEntityId::from_raw(100);
const NPC: StableEntityId = StableEntityId::from_raw(200);
const RUNTIME: StableEntityId = StableEntityId::from_raw(300);

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
    .init_resource::<ChunkLifecycle>()
    .init_resource::<ChunkLifecycleConfig>()
    .init_resource::<ChunkLifecycleRequests>()
    .init_resource::<ChunkLifecycleReport>()
    .add_systems(Update, process_chunk_lifecycle_requests)
    .persist_component_as::<SavedDoor>("test.door.v1")
    .persist_component_as::<SavedHealth>("test.health.v1");
    app
}

#[test]
fn lifecycle_requests_reject_invalid_chunk_ids() {
    let mut requests = ChunkLifecycleRequests::default();

    let err = requests.request_load(ChunkId::INVALID).unwrap_err();

    assert_eq!(err, ChunkLifecycleRequestError::InvalidChunkId);
}

#[test]
fn lifecycle_load_spawn_activate_and_sleep_are_idempotent() {
    let mut app = app();

    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_load(CHUNK)
        .unwrap();
    app.update();
    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_load(CHUNK)
        .unwrap();
    app.update();
    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_spawned(CHUNK)
        .unwrap();
    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_activate(CHUNK)
        .unwrap();
    app.update();
    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_sleep(CHUNK)
        .unwrap();
    app.update();

    let lifecycle = app.world().resource::<ChunkLifecycle>();
    assert_eq!(lifecycle.state(CHUNK), ChunkLifecycleState::Sleeping);
    assert_eq!(lifecycle.chunks().len(), 1);
}

#[test]
fn lifecycle_ignores_requests_that_would_downgrade_loaded_chunks() {
    let mut app = app();
    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_load(CHUNK)
        .unwrap();
    app.update();
    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_spawned(CHUNK)
        .unwrap();
    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_activate(CHUNK)
        .unwrap();
    app.update();

    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_load(CHUNK)
        .unwrap();
    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_spawned(CHUNK)
        .unwrap();
    app.update();

    let lifecycle = app.world().resource::<ChunkLifecycle>();
    let report = app.world().resource::<ChunkLifecycleReport>();
    assert_eq!(lifecycle.state(CHUNK), ChunkLifecycleState::GameplayActive);
    assert!(report.transitions.is_empty());
}

#[test]
fn lifecycle_ignores_unload_for_unloaded_chunks_without_saving_empty_delta() {
    let mut app = app();

    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_unload(CHUNK)
        .unwrap();
    app.update();

    let report = app.world().resource::<ChunkLifecycleReport>();
    assert_eq!(
        app.world().resource::<ChunkLifecycle>().state(CHUNK),
        ChunkLifecycleState::Unloaded
    );
    assert!(report.transitions.is_empty());
    assert!(report.saved_chunks.is_empty());
    assert!(
        app.world()
            .resource::<PersistentWorldDeltas>()
            .get(CHUNK)
            .is_none()
    );
}

#[test]
fn lifecycle_does_not_reapply_saved_delta_for_duplicate_spawned_request() {
    let mut app = app();
    app.world_mut()
        .resource_mut::<PersistentWorldDeltas>()
        .insert(saved_chunk_delta());
    app.world_mut().spawn((
        Persistent,
        DOOR,
        ChunkMembership::new(CHUNK),
        SavedDoor { open: false },
    ));
    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_load(CHUNK)
        .unwrap();
    app.update();
    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_spawned(CHUNK)
        .unwrap();
    app.update();

    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_spawned(CHUNK)
        .unwrap();
    app.update();

    let report = app.world().resource::<ChunkLifecycleReport>();
    assert!(report.applied_saved_chunks.is_empty());
    assert!(report.transitions.is_empty());
}

#[test]
fn lifecycle_unload_saves_persistent_state_and_despawns_only_that_chunk() {
    let mut app = app();
    spawn_chunk_entities(app.world_mut());
    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_load(CHUNK)
        .unwrap();
    app.update();
    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_spawned(CHUNK)
        .unwrap();
    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_activate(CHUNK)
        .unwrap();
    app.update();

    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_unload(CHUNK)
        .unwrap();
    app.update();

    let lifecycle = app.world().resource::<ChunkLifecycle>();
    let report = app.world().resource::<ChunkLifecycleReport>();
    let registry = app.world().resource::<StableEntityRegistry>();
    let saved = app
        .world()
        .resource::<PersistentWorldDeltas>()
        .get(CHUNK)
        .unwrap();
    assert_eq!(lifecycle.state(CHUNK), ChunkLifecycleState::Unloaded);
    assert_eq!(report.saved_chunks, [CHUNK]);
    assert_eq!(report.despawned_entities, 2);
    assert!(registry.entity(DOOR).is_none());
    assert!(registry.entity(RUNTIME).is_none());
    assert!(registry.entity(NPC).is_some());
    assert_eq!(saved.entities.len(), 1);
    assert_eq!(saved.entities[0].entity, DOOR);
}

#[test]
fn lifecycle_spawned_applies_saved_delta_after_baseline_spawn() {
    let mut app = app();
    app.world_mut()
        .resource_mut::<PersistentWorldDeltas>()
        .insert(saved_chunk_delta());
    app.world_mut().spawn((
        Persistent,
        DOOR,
        ChunkMembership::new(CHUNK),
        SavedDoor { open: false },
    ));
    app.world_mut().spawn((
        Persistent,
        NPC,
        ChunkMembership::new(CHUNK),
        SavedHealth { current: 100 },
    ));

    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_load(CHUNK)
        .unwrap();
    app.update();
    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_spawned(CHUNK)
        .unwrap();
    app.update();

    let lifecycle = app.world().resource::<ChunkLifecycle>();
    let report = app.world().resource::<ChunkLifecycleReport>();
    let registry = app.world().resource::<StableEntityRegistry>();
    let door = registry.entity(DOOR).unwrap();
    assert_eq!(lifecycle.state(CHUNK), ChunkLifecycleState::Spawned);
    assert_eq!(report.applied_saved_chunks, [CHUNK]);
    assert_eq!(
        app.world().get::<SavedDoor>(door),
        Some(&SavedDoor { open: true })
    );
    assert!(registry.entity(NPC).is_none());
}

#[test]
fn lifecycle_spawned_reports_invalid_saved_delta_without_panicking() {
    let mut app = app();
    app.world_mut()
        .resource_mut::<PersistentWorldDeltas>()
        .insert(ChunkPersistentDelta {
            chunk: CHUNK,
            entities: vec![PersistentEntityDelta {
                entity: DOOR,
                components: vec![PersistentComponentValue {
                    type_name: "test.unknown.v1".into(),
                    payload: Vec::new(),
                }],
                removed_components: Vec::new(),
            }],
            deleted: Vec::new(),
        });

    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_spawned(CHUNK)
        .unwrap();
    app.update();

    let report = app.world().resource::<ChunkLifecycleReport>();
    assert_eq!(
        app.world().resource::<ChunkLifecycle>().state(CHUNK),
        ChunkLifecycleState::Unloaded
    );
    assert_eq!(report.applied_saved_chunks, []);
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].chunk, CHUNK);
    assert!(report.errors[0].message.contains("not registered"));
}

#[test]
fn lifecycle_does_not_activate_chunk_when_spawn_apply_fails() {
    let mut app = app();
    app.world_mut()
        .resource_mut::<PersistentWorldDeltas>()
        .insert(ChunkPersistentDelta {
            chunk: CHUNK,
            entities: vec![PersistentEntityDelta {
                entity: DOOR,
                components: vec![PersistentComponentValue {
                    type_name: "test.unknown.v1".into(),
                    payload: Vec::new(),
                }],
                removed_components: Vec::new(),
            }],
            deleted: Vec::new(),
        });
    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_load(CHUNK)
        .unwrap();
    app.update();

    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_spawned(CHUNK)
        .unwrap();
    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_activate(CHUNK)
        .unwrap();
    app.update();

    assert_eq!(
        app.world().resource::<ChunkLifecycle>().state(CHUNK),
        ChunkLifecycleState::Loading
    );
    assert_eq!(
        app.world().resource::<ChunkLifecycleReport>().errors.len(),
        1
    );
}

#[test]
fn lifecycle_unload_can_skip_automatic_save_with_config_knob() {
    let mut app = app();
    spawn_chunk_entities(app.world_mut());
    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_load(CHUNK)
        .unwrap();
    app.update();
    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_spawned(CHUNK)
        .unwrap();
    app.update();
    app.world_mut()
        .resource_mut::<ChunkLifecycleConfig>()
        .save_on_unload = false;

    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_unload(CHUNK)
        .unwrap();
    app.update();

    assert!(
        app.world()
            .resource::<PersistentWorldDeltas>()
            .get(CHUNK)
            .is_none()
    );
    assert_eq!(
        app.world().resource::<ChunkLifecycle>().state(CHUNK),
        ChunkLifecycleState::Unloaded
    );
    assert_eq!(
        app.world().resource::<ChunkLifecycleReport>().saved_chunks,
        []
    );
    assert_eq!(
        app.world()
            .resource::<ChunkLifecycleReport>()
            .despawned_entities,
        2
    );
}

#[test]
fn lifecycle_unload_during_loading_cleans_partial_entities_without_saving() {
    let mut app = app();
    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_load(CHUNK)
        .unwrap();
    app.update();
    app.world_mut().spawn((
        Persistent,
        DOOR,
        ChunkMembership::new(CHUNK),
        SavedDoor { open: false },
    ));

    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_unload(CHUNK)
        .unwrap();
    app.update();

    let report = app.world().resource::<ChunkLifecycleReport>();
    let registry = app.world().resource::<StableEntityRegistry>();
    assert_eq!(
        app.world().resource::<ChunkLifecycle>().state(CHUNK),
        ChunkLifecycleState::Unloaded
    );
    assert!(report.saved_chunks.is_empty());
    assert_eq!(report.despawned_entities, 1);
    assert!(registry.entity(DOOR).is_none());
    assert!(
        app.world()
            .resource::<PersistentWorldDeltas>()
            .get(CHUNK)
            .is_none()
    );
}

fn spawn_chunk_entities(world: &mut World) {
    world.spawn((
        Persistent,
        DOOR,
        ChunkMembership::new(CHUNK),
        SavedDoor { open: true },
    ));
    world.spawn((RuntimeOnly, RUNTIME, ChunkMembership::new(CHUNK)));
    world.spawn((
        Persistent,
        NPC,
        ChunkMembership::new(OTHER_CHUNK),
        SavedHealth { current: 100 },
    ));
}

fn saved_chunk_delta() -> ChunkPersistentDelta {
    ChunkPersistentDelta {
        chunk: CHUNK,
        entities: vec![PersistentEntityDelta {
            entity: DOOR,
            components: vec![PersistentComponentValue {
                type_name: "test.door.v1".into(),
                payload: serde_json::to_vec(&SavedDoor { open: true }).unwrap(),
            }],
            removed_components: vec!["test.door.v1".into()],
        }],
        deleted: vec![NPC],
    }
}
