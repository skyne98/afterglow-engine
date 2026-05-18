use super::*;
use crate::core::AfterglowCorePlugin;
use serde::{Deserialize, Serialize};

const CHUNK: ChunkId = ChunkId::from_raw(7);
const OTHER_CHUNK: ChunkId = ChunkId::from_raw(8);
const FIRST: StableEntityId = StableEntityId::from_raw(100);
const SECOND: StableEntityId = StableEntityId::from_raw(200);

#[derive(Component, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct SavedHealth {
    current: u32,
}

#[derive(Component, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct SavedDoor {
    open: bool,
}

#[derive(Component)]
struct NotSaved;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AfterglowCorePlugin,
        AfterglowPersistencePlugin,
    ))
    .persist_component_as::<SavedHealth>("test.health.v1")
    .persist_component_as::<SavedDoor>("test.door.v1");
    app
}

fn unregistered_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AfterglowCorePlugin,
        AfterglowPersistencePlugin,
    ));
    app
}

#[test]
fn registering_components_populates_persistence_registry_once_per_type() {
    let mut app = app();
    app.persist_component_as::<SavedHealth>("test.health.v1");

    let registry = app.world().resource::<PersistenceRegistry>();
    assert_eq!(registry.len(), 2);
    assert!(
        registry
            .component_names()
            .contains(&"test.health.v1".to_string())
    );
}

#[test]
fn capture_chunk_delta_records_registered_components_only_for_persistent_chunk_entities() {
    let mut app = app();
    app.world_mut().spawn((
        Persistent,
        FIRST,
        ChunkMembership::new(CHUNK),
        SavedHealth { current: 10 },
        SavedDoor { open: true },
        NotSaved,
    ));
    app.world_mut().spawn((
        Persistent,
        SECOND,
        ChunkMembership::new(OTHER_CHUNK),
        SavedHealth { current: 99 },
    ));
    app.world_mut().spawn((
        StableEntityId::from_raw(300),
        ChunkMembership::new(CHUNK),
        SavedHealth { current: 1 },
    ));

    let delta = capture_chunk_delta(app.world_mut(), CHUNK).unwrap();

    assert_eq!(delta.chunk, CHUNK);
    assert_eq!(delta.entities.len(), 1);
    assert_eq!(delta.entities[0].entity, FIRST);
    assert_eq!(delta.entities[0].components.len(), 2);
    assert_eq!(
        component::<SavedHealth>(&delta.entities[0]).unwrap(),
        SavedHealth { current: 10 }
    );
    assert_eq!(
        component::<SavedDoor>(&delta.entities[0]).unwrap(),
        SavedDoor { open: true }
    );
}

#[test]
fn capture_chunk_deltas_groups_multiple_chunks_in_one_pass() {
    let mut app = app();
    app.world_mut().spawn((
        Persistent,
        FIRST,
        ChunkMembership::new(CHUNK),
        SavedHealth { current: 10 },
    ));
    app.world_mut().spawn((
        Persistent,
        SECOND,
        ChunkMembership::new(OTHER_CHUNK),
        SavedHealth { current: 20 },
    ));

    let deltas = capture_chunk_deltas(app.world_mut(), [CHUNK, OTHER_CHUNK]).unwrap();

    assert_eq!(deltas.len(), 2);
    assert_eq!(deltas[0].chunk, CHUNK);
    assert_eq!(deltas[0].entities[0].entity, FIRST);
    assert_eq!(
        component::<SavedHealth>(&deltas[0].entities[0]).unwrap(),
        SavedHealth { current: 10 }
    );
    assert_eq!(deltas[1].chunk, OTHER_CHUNK);
    assert_eq!(deltas[1].entities[0].entity, SECOND);
    assert_eq!(
        component::<SavedHealth>(&deltas[1].entities[0]).unwrap(),
        SavedHealth { current: 20 }
    );
}

#[test]
fn apply_chunk_delta_updates_existing_entity_components_and_removes_absent_registered_components() {
    let mut app = app();
    let entity = app
        .world_mut()
        .spawn((
            Persistent,
            FIRST,
            ChunkMembership::new(OTHER_CHUNK),
            SavedHealth { current: 1 },
            SavedDoor { open: false },
        ))
        .id();
    let delta = ChunkPersistentDelta {
        chunk: CHUNK,
        entities: vec![PersistentEntityDelta {
            entity: FIRST,
            components: vec![value(&SavedHealth { current: 42 })],
            removed_components: vec!["test.health.v1".into(), "test.door.v1".into()],
        }],
        deleted: Vec::new(),
    };

    let report = apply_chunk_delta(app.world_mut(), &delta).unwrap();

    assert_eq!(report.updated, 1);
    assert_eq!(report.components_applied, 1);
    assert_eq!(report.components_removed, 1);
    assert_eq!(
        app.world().get::<SavedHealth>(entity),
        Some(&SavedHealth { current: 42 })
    );
    assert!(app.world().get::<SavedDoor>(entity).is_none());
    assert_eq!(
        app.world().get::<ChunkMembership>(entity),
        Some(&ChunkMembership::new(CHUNK))
    );
}

#[test]
fn capture_chunk_delta_records_entities_with_only_removed_registered_components() {
    let mut app = app();
    app.world_mut()
        .spawn((Persistent, FIRST, ChunkMembership::new(CHUNK)));

    let delta = capture_chunk_delta(app.world_mut(), CHUNK).unwrap();

    assert_eq!(delta.entities.len(), 1);
    assert_eq!(delta.entities[0].entity, FIRST);
    assert!(delta.entities[0].components.is_empty());
    assert_eq!(
        delta.entities[0].removed_components,
        ["test.door.v1".to_string(), "test.health.v1".to_string()]
    );
}

#[test]
fn capture_chunk_delta_is_empty_when_no_components_are_registered() {
    let mut app = unregistered_app();
    app.world_mut()
        .spawn((Persistent, FIRST, ChunkMembership::new(CHUNK)));

    let delta = capture_chunk_delta(app.world_mut(), CHUNK).unwrap();

    assert!(delta.entities.is_empty());
    assert!(delta.deleted.is_empty());
}

#[test]
fn apply_chunk_delta_spawns_missing_entity_with_registered_components() {
    let mut app = app();
    let delta = ChunkPersistentDelta {
        chunk: CHUNK,
        entities: vec![PersistentEntityDelta {
            entity: FIRST,
            components: vec![value(&SavedDoor { open: true })],
            removed_components: vec!["test.door.v1".into()],
        }],
        deleted: Vec::new(),
    };

    let report = apply_chunk_delta(app.world_mut(), &delta).unwrap();

    assert_eq!(report.spawned, 1);
    let entity = app
        .world()
        .resource::<StableEntityRegistry>()
        .entity(FIRST)
        .unwrap();
    assert!(app.world().get::<Persistent>(entity).is_some());
    assert_eq!(
        app.world().get::<SavedDoor>(entity),
        Some(&SavedDoor { open: true })
    );
}

#[test]
fn apply_chunk_deltas_restores_multiple_chunks_in_one_pass() {
    let mut app = app();
    let deltas = vec![
        ChunkPersistentDelta {
            chunk: CHUNK,
            entities: vec![PersistentEntityDelta {
                entity: FIRST,
                components: vec![value(&SavedHealth { current: 11 })],
                removed_components: vec!["test.health.v1".into()],
            }],
            deleted: Vec::new(),
        },
        ChunkPersistentDelta {
            chunk: OTHER_CHUNK,
            entities: vec![PersistentEntityDelta {
                entity: SECOND,
                components: vec![value(&SavedHealth { current: 22 })],
                removed_components: vec!["test.health.v1".into()],
            }],
            deleted: Vec::new(),
        },
    ];

    let report = apply_chunk_deltas(app.world_mut(), &deltas).unwrap();

    assert_eq!(report.spawned, 2);
    let registry = app.world().resource::<StableEntityRegistry>();
    let first = registry.entity(FIRST).unwrap();
    let second = registry.entity(SECOND).unwrap();
    assert_eq!(
        app.world().get::<SavedHealth>(first),
        Some(&SavedHealth { current: 11 })
    );
    assert_eq!(
        app.world().get::<SavedHealth>(second),
        Some(&SavedHealth { current: 22 })
    );
    assert_eq!(
        app.world().get::<ChunkMembership>(second),
        Some(&ChunkMembership::new(OTHER_CHUNK))
    );
}

#[test]
fn apply_chunk_delta_rejects_unregistered_component_payloads() {
    let mut app = app();
    let delta = ChunkPersistentDelta {
        chunk: CHUNK,
        entities: vec![PersistentEntityDelta {
            entity: FIRST,
            components: vec![PersistentComponentValue {
                type_name: "game::Unknown".into(),
                payload: Vec::new(),
            }],
            removed_components: Vec::new(),
        }],
        deleted: Vec::new(),
    };

    let err = apply_chunk_delta(app.world_mut(), &delta).unwrap_err();
    assert!(
        app.world()
            .resource::<StableEntityRegistry>()
            .entity(FIRST)
            .is_none()
    );

    assert!(matches!(
        err,
        PersistenceError::UnregisteredComponent { type_name } if type_name == "game::Unknown"
    ));
}

#[test]
fn apply_chunk_delta_rejects_malformed_payloads_before_mutating_world() {
    let mut app = app();
    let first = app
        .world_mut()
        .spawn((
            Persistent,
            FIRST,
            ChunkMembership::new(CHUNK),
            SavedHealth { current: 5 },
        ))
        .id();
    let second = app
        .world_mut()
        .spawn((Persistent, SECOND, ChunkMembership::new(CHUNK)))
        .id();
    let delta = ChunkPersistentDelta {
        chunk: CHUNK,
        entities: vec![PersistentEntityDelta {
            entity: FIRST,
            components: vec![PersistentComponentValue {
                type_name: "test.health.v1".into(),
                payload: b"not json".to_vec(),
            }],
            removed_components: Vec::new(),
        }],
        deleted: vec![SECOND],
    };

    let err = apply_chunk_delta(app.world_mut(), &delta).unwrap_err();

    assert!(matches!(
        err,
        PersistenceError::Deserialize { type_name, .. } if type_name == "test.health.v1"
    ));
    assert_eq!(
        app.world().get::<SavedHealth>(first),
        Some(&SavedHealth { current: 5 })
    );
    assert!(app.world().get_entity(second).is_ok());
}

#[test]
fn tombstones_only_despawn_entities_in_the_delta_chunk() {
    let mut app = app();
    let entity = app
        .world_mut()
        .spawn((
            Persistent,
            FIRST,
            ChunkMembership::new(OTHER_CHUNK),
            SavedHealth { current: 3 },
        ))
        .id();
    let delta = ChunkPersistentDelta {
        chunk: CHUNK,
        entities: Vec::new(),
        deleted: vec![FIRST],
    };

    let report = apply_chunk_delta(app.world_mut(), &delta).unwrap();

    assert_eq!(report.despawned, 0);
    assert!(app.world().get_entity(entity).is_ok());
}

#[test]
fn persistent_delta_store_roundtrips_json_and_replaces_chunks() {
    let mut store = PersistentWorldDeltas::default();
    let first = ChunkPersistentDelta {
        chunk: CHUNK,
        entities: vec![PersistentEntityDelta {
            entity: FIRST,
            components: vec![value(&SavedHealth { current: 1 })],
            removed_components: vec!["test.health.v1".into()],
        }],
        deleted: Vec::new(),
    };
    let second = ChunkPersistentDelta {
        chunk: CHUNK,
        entities: Vec::new(),
        deleted: vec![SECOND],
    };

    assert!(store.insert(first).is_none());
    assert!(store.insert(second.clone()).is_some());
    let bytes = store.to_json().unwrap();
    let decoded = PersistentWorldDeltas::from_json(&bytes).unwrap();

    assert_eq!(decoded.get(CHUNK), Some(&second));
}

fn value<T>(component: &T) -> PersistentComponentValue
where
    T: Serialize,
{
    PersistentComponentValue {
        type_name: stable_name::<T>().into(),
        payload: serde_json::to_vec(component).unwrap(),
    }
}

fn component<T>(delta: &PersistentEntityDelta) -> Option<T>
where
    T: DeserializeOwned,
{
    delta
        .components
        .iter()
        .find(|component| component.type_name == stable_name::<T>())
        .map(|component| serde_json::from_slice(&component.payload).unwrap())
}

fn stable_name<T>() -> &'static str {
    match type_name::<T>() {
        name if name == type_name::<SavedHealth>() => "test.health.v1",
        name if name == type_name::<SavedDoor>() => "test.door.v1",
        name => unreachable!("missing test stable name for {name}"),
    }
}
