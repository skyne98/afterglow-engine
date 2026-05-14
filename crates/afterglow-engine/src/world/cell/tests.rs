use super::*;
use crate::{
    core::AfterglowCorePlugin,
    persistence::{
        AfterglowPersistencePlugin, ChunkPersistentDelta, PersistenceAppExt,
        PersistentComponentValue, PersistentEntityDelta, PersistentWorldDeltas,
    },
    world::{
        AfterglowWorldPlugin,
        lifecycle::{ChunkLifecycleConfig, ChunkLifecycleRequests},
    },
};
use serde::{Deserialize, Serialize};

const TEST_CHUNK: ChunkId = ChunkId::from_raw(42);
const TEST_ENTITY: StableEntityId = StableEntityId::from_raw(4_200);
const TEST_ENTITY_2: StableEntityId = StableEntityId::from_raw(4_201);
const OTHER_CHUNK: ChunkId = ChunkId::from_raw(43);

#[derive(Component, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct SavedDoor {
    open: bool,
}

fn app_without_demo() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        AfterglowCorePlugin,
        AfterglowPersistencePlugin,
    ))
    .init_resource::<Assets<Mesh>>()
    .init_resource::<Assets<StandardMaterial>>()
    .init_resource::<CellManifestRegistry>()
    .init_resource::<CellLoadRequests>()
    .init_resource::<CellLoadTracker>()
    .init_resource::<CellLoadReport>()
    .init_resource::<crate::world::lifecycle::ChunkLifecycle>()
    .init_resource::<ChunkLifecycleConfig>()
    .init_resource::<ChunkLifecycleRequests>()
    .add_systems(
        Update,
        (
            process_cell_load_requests,
            crate::world::lifecycle::process_chunk_lifecycle_requests,
        )
            .chain(),
    )
    .persist_component_as::<SavedDoor>("test.door.v1");
    app
}

#[test]
fn cell_load_request_flows_through_lifecycle_and_spawns_manifest() {
    let mut app = app_without_demo();
    register_test_manifest(&mut app);

    app.world_mut()
        .resource_mut::<CellLoadRequests>()
        .request_load(TEST_CHUNK)
        .unwrap();
    app.update();

    assert_eq!(
        app.world()
            .resource::<crate::world::lifecycle::ChunkLifecycle>()
            .state(TEST_CHUNK),
        ChunkLifecycleState::Loading
    );
    assert!(app.world().resource::<StableEntityRegistry>().is_empty());

    app.update();

    let registry = app.world().resource::<StableEntityRegistry>();
    let report = app.world().resource::<CellLoadReport>();
    assert_eq!(report.spawned_chunks, [TEST_CHUNK]);
    assert_eq!(report.spawned_entities, 1);
    assert_eq!(registry.chunk_entities(TEST_CHUNK).len(), 1);
    assert!(registry.entity(TEST_ENTITY).is_some());
    assert_eq!(
        app.world()
            .resource::<crate::world::lifecycle::ChunkLifecycle>()
            .state(TEST_CHUNK),
        ChunkLifecycleState::Spawned
    );
}

#[test]
fn cell_load_is_idempotent_after_completion() {
    let mut app = app_without_demo();
    register_test_manifest(&mut app);
    request_and_finish_test_cell_load(&mut app);

    app.world_mut()
        .resource_mut::<CellLoadRequests>()
        .request_load(TEST_CHUNK)
        .unwrap();
    app.update();

    let registry = app.world().resource::<StableEntityRegistry>();
    let report = app.world().resource::<CellLoadReport>();
    assert_eq!(registry.chunk_entities(TEST_CHUNK).len(), 1);
    assert_eq!(report.completed_chunks, [TEST_CHUNK]);
    assert_eq!(report.spawned_entities, 0);
}

#[test]
fn cell_load_applies_persistent_delta_after_baseline_spawn() {
    let mut app = app_without_demo();
    register_test_manifest(&mut app);
    app.world_mut()
        .resource_mut::<PersistentWorldDeltas>()
        .insert(ChunkPersistentDelta {
            chunk: TEST_CHUNK,
            entities: vec![PersistentEntityDelta {
                entity: TEST_ENTITY,
                components: vec![PersistentComponentValue {
                    type_name: "test.door.v1".into(),
                    payload: serde_json::to_vec(&SavedDoor { open: true }).unwrap(),
                }],
                removed_components: Vec::new(),
            }],
            deleted: Vec::new(),
        });

    request_and_finish_test_cell_load(&mut app);

    let registry = app.world().resource::<StableEntityRegistry>();
    let entity = registry.entity(TEST_ENTITY).unwrap();
    assert_eq!(
        app.world().get::<SavedDoor>(entity),
        Some(&SavedDoor { open: true })
    );
}

#[test]
fn cell_load_retries_spawned_request_without_duplicate_baseline_after_delta_failure() {
    let mut app = app_without_demo();
    register_test_manifest(&mut app);
    app.world_mut()
        .resource_mut::<PersistentWorldDeltas>()
        .insert(ChunkPersistentDelta {
            chunk: TEST_CHUNK,
            entities: vec![PersistentEntityDelta {
                entity: TEST_ENTITY,
                components: vec![PersistentComponentValue {
                    type_name: "test.missing.v1".into(),
                    payload: Vec::new(),
                }],
                removed_components: Vec::new(),
            }],
            deleted: Vec::new(),
        });

    app.world_mut()
        .resource_mut::<CellLoadRequests>()
        .request_load(TEST_CHUNK)
        .unwrap();
    app.update();
    app.update();
    app.update();

    let registry = app.world().resource::<StableEntityRegistry>();
    assert_eq!(registry.chunk_entities(TEST_CHUNK).len(), 1);
    assert_eq!(
        app.world()
            .resource::<crate::world::lifecycle::ChunkLifecycle>()
            .state(TEST_CHUNK),
        ChunkLifecycleState::Loading
    );
    assert_eq!(
        app.world().resource::<CellLoadRequests>().pending(),
        &BTreeSet::from([TEST_CHUNK])
    );
}

#[test]
fn cell_load_reports_missing_manifest_and_drops_request() {
    let mut app = app_without_demo();

    app.world_mut()
        .resource_mut::<CellLoadRequests>()
        .request_load(TEST_CHUNK)
        .unwrap();
    app.update();

    let report = app.world().resource::<CellLoadReport>();
    assert_eq!(report.missing_chunks, [TEST_CHUNK]);
    assert!(
        app.world()
            .resource::<CellLoadRequests>()
            .pending()
            .is_empty()
    );
}

#[test]
fn cell_manifest_registry_rejects_invalid_or_duplicate_identity() {
    let mut registry = CellManifestRegistry::default();
    let mut manifest = test_manifest();
    manifest.entities.push(manifest.entities[0].clone());

    let err = registry.insert(manifest).unwrap_err();

    assert_eq!(err, CellManifestError::DuplicateStableEntityId(TEST_ENTITY));
}

#[test]
fn cell_load_reloads_manifest_after_lifecycle_unload() {
    let mut app = app_without_demo();
    register_test_manifest(&mut app);
    request_and_finish_test_cell_load(&mut app);

    app.world_mut()
        .resource_mut::<ChunkLifecycleRequests>()
        .request_unload(TEST_CHUNK)
        .unwrap();
    app.update();
    assert!(
        app.world()
            .resource::<StableEntityRegistry>()
            .chunk_entities(TEST_CHUNK)
            .is_empty()
    );

    app.world_mut()
        .resource_mut::<CellLoadRequests>()
        .request_load(TEST_CHUNK)
        .unwrap();
    app.update();
    app.update();

    assert_eq!(
        app.world()
            .resource::<StableEntityRegistry>()
            .chunk_entities(TEST_CHUNK)
            .len(),
        1
    );
}

#[test]
fn world_plugin_loads_built_in_demo_cell_through_manifest_path() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        AfterglowCorePlugin,
        AfterglowPersistencePlugin,
        AfterglowWorldPlugin,
    ))
    .init_resource::<Assets<Mesh>>()
    .init_resource::<Assets<StandardMaterial>>();

    app.update();
    app.update();

    let registry = app.world().resource::<StableEntityRegistry>();
    assert_eq!(registry.chunk_entities(DEMO_CELL_CHUNK).len(), 3);
    assert!(registry.entity(DEMO_CUBE_ID).is_some());
    assert!(registry.entity(DEMO_LIGHT_ID).is_some());
    assert!(registry.entity(DEMO_CAMERA_ID).is_some());
}

#[test]
fn world_plugin_keeps_user_supplied_cell_resources() {
    let mut app = App::new();
    let mut registry = CellManifestRegistry::default();
    registry.insert(test_manifest()).unwrap();
    app.insert_resource(registry);
    app.insert_resource(CellLoadRequests::default());

    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        AfterglowCorePlugin,
        AfterglowPersistencePlugin,
        AfterglowWorldPlugin,
    ));

    assert!(
        app.world()
            .resource::<CellManifestRegistry>()
            .contains(TEST_CHUNK)
    );
    assert!(
        !app.world()
            .resource::<CellManifestRegistry>()
            .contains(DEMO_CELL_CHUNK)
    );
    assert!(
        app.world()
            .resource::<CellLoadRequests>()
            .pending()
            .is_empty()
    );
}

#[test]
fn world_plugin_does_not_demo_autoload_user_supplied_registry() {
    let mut app = App::new();
    let mut registry = CellManifestRegistry::default();
    registry.insert(test_manifest()).unwrap();
    app.insert_resource(registry);

    app.add_plugins((
        MinimalPlugins,
        AfterglowCorePlugin,
        AfterglowPersistencePlugin,
        AfterglowWorldPlugin,
    ));

    assert!(
        app.world()
            .resource::<CellLoadRequests>()
            .pending()
            .is_empty()
    );
}

#[test]
fn cell_load_missing_asset_resources_does_not_spawn_partial_baseline() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AfterglowCorePlugin,
        AfterglowPersistencePlugin,
    ))
    .insert_resource(CellManifestRegistry::with_demo_cell())
    .insert_resource(CellLoadRequests::with_demo_cell())
    .init_resource::<CellLoadTracker>()
    .init_resource::<CellLoadReport>()
    .init_resource::<crate::world::lifecycle::ChunkLifecycle>()
    .init_resource::<ChunkLifecycleConfig>()
    .init_resource::<ChunkLifecycleRequests>()
    .add_systems(
        Update,
        (
            process_cell_load_requests,
            crate::world::lifecycle::process_chunk_lifecycle_requests,
        )
            .chain(),
    );

    app.update();
    app.update();

    let report = app.world().resource::<CellLoadReport>();
    assert_eq!(report.errors.len(), 1);
    assert!(report.errors[0].message.contains("Assets<Mesh>"));
    assert!(
        app.world()
            .resource::<StableEntityRegistry>()
            .chunk_entities(DEMO_CELL_CHUNK)
            .is_empty()
    );
}

#[test]
fn cell_load_errors_when_stable_id_belongs_to_another_chunk() {
    let mut app = app_without_demo();
    register_test_manifest(&mut app);
    app.world_mut()
        .spawn((TEST_ENTITY, ChunkMembership::new(OTHER_CHUNK)));

    app.world_mut()
        .resource_mut::<CellLoadRequests>()
        .request_load(TEST_CHUNK)
        .unwrap();
    app.update();
    app.update();

    let report = app.world().resource::<CellLoadReport>();
    assert_eq!(report.errors.len(), 1);
    assert!(report.errors[0].message.contains("already belongs"));
}

#[test]
fn cell_load_conflict_preflight_prevents_partial_baseline_spawn() {
    let mut app = app_without_demo();
    let mut manifest = test_manifest();
    manifest.entities.push(CellEntityTemplate {
        stable_id: TEST_ENTITY_2,
        name: Some("Conflicting Entity".into()),
        persistent: true,
        transform: Transform::default(),
        kind: CellEntityKind::Empty,
    });
    app.world_mut()
        .resource_mut::<CellManifestRegistry>()
        .insert(manifest)
        .unwrap();
    app.world_mut()
        .spawn((TEST_ENTITY_2, ChunkMembership::new(OTHER_CHUNK)));

    app.world_mut()
        .resource_mut::<CellLoadRequests>()
        .request_load(TEST_CHUNK)
        .unwrap();
    app.update();
    app.update();

    let registry = app.world().resource::<StableEntityRegistry>();
    let report = app.world().resource::<CellLoadReport>();
    assert_eq!(report.errors.len(), 1);
    assert!(registry.entity(TEST_ENTITY).is_none());
    assert_eq!(registry.chunk_entities(TEST_CHUNK).len(), 0);
}

#[test]
fn cell_load_replaces_previous_builtin_template_components() {
    let mut app = app_without_demo();
    register_test_manifest(&mut app);
    let entity = app
        .world_mut()
        .spawn((
            TEST_ENTITY,
            ChunkMembership::new(TEST_CHUNK),
            PointLight::default(),
        ))
        .id();

    request_and_finish_test_cell_load(&mut app);

    assert!(app.world().get::<PointLight>(entity).is_none());
    assert_eq!(
        app.world().get::<ChunkMembership>(entity),
        Some(&ChunkMembership::new(TEST_CHUNK))
    );
}

fn register_test_manifest(app: &mut App) {
    app.world_mut()
        .resource_mut::<CellManifestRegistry>()
        .insert(test_manifest())
        .unwrap();
}

fn request_and_finish_test_cell_load(app: &mut App) {
    app.world_mut()
        .resource_mut::<CellLoadRequests>()
        .request_load(TEST_CHUNK)
        .unwrap();
    app.update();
    app.update();
    app.update();
}

fn test_manifest() -> CellManifest {
    CellManifest {
        chunk: TEST_CHUNK,
        entities: vec![CellEntityTemplate {
            stable_id: TEST_ENTITY,
            name: Some("Test Door".into()),
            persistent: true,
            transform: Transform::from_xyz(1.0, 2.0, 3.0),
            kind: CellEntityKind::Empty,
        }],
    }
}
