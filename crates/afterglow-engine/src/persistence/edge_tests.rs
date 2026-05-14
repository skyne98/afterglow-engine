use super::*;
use crate::core::{AfterglowCorePlugin, identity::StableEntityRegistry};
use serde::{Deserialize, Serialize};

const CHUNK: ChunkId = ChunkId::from_raw(7);
const OTHER_CHUNK: ChunkId = ChunkId::from_raw(8);
const FIRST: StableEntityId = StableEntityId::from_raw(100);

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
    .persist_component_as::<SavedHealth>("test.health.v1");
    app
}

#[test]
fn apply_chunk_delta_rejects_invalid_entity_ids_before_mutating_world() {
    let mut app = app();
    let existing = app
        .world_mut()
        .spawn((
            Persistent,
            FIRST,
            ChunkMembership::new(CHUNK),
            SavedHealth { current: 5 },
        ))
        .id();
    let delta = ChunkPersistentDelta {
        chunk: CHUNK,
        entities: vec![PersistentEntityDelta {
            entity: StableEntityId::INVALID,
            components: vec![value(&SavedHealth { current: 99 })],
            removed_components: Vec::new(),
        }],
        deleted: vec![FIRST],
    };

    let err = apply_chunk_delta(app.world_mut(), &delta).unwrap_err();

    assert!(matches!(err, PersistenceError::InvalidEntityId));
    assert_eq!(
        app.world().get::<SavedHealth>(existing),
        Some(&SavedHealth { current: 5 })
    );
    assert!(app.world().get_entity(existing).is_ok());
}

#[test]
fn capture_chunk_delta_rejects_invalid_chunk_ids() {
    let mut app = app();
    app.world_mut().spawn((
        Persistent,
        FIRST,
        ChunkMembership::new(CHUNK),
        SavedHealth { current: 5 },
    ));

    let err = capture_chunk_delta(app.world_mut(), ChunkId::INVALID).unwrap_err();

    assert!(matches!(err, PersistenceError::InvalidChunkId));
}

#[test]
fn apply_chunk_delta_rejects_unregistered_removed_components_before_mutating_world() {
    let mut app = app();
    let existing = app
        .world_mut()
        .spawn((
            Persistent,
            FIRST,
            ChunkMembership::new(CHUNK),
            SavedHealth { current: 5 },
        ))
        .id();
    let delta = ChunkPersistentDelta {
        chunk: CHUNK,
        entities: vec![PersistentEntityDelta {
            entity: FIRST,
            components: Vec::new(),
            removed_components: vec!["game.unknown.v1".into()],
        }],
        deleted: Vec::new(),
    };

    let err = apply_chunk_delta(app.world_mut(), &delta).unwrap_err();

    assert!(matches!(
        err,
        PersistenceError::UnregisteredComponent { type_name } if type_name == "game.unknown.v1"
    ));
    assert_eq!(
        app.world().get::<SavedHealth>(existing),
        Some(&SavedHealth { current: 5 })
    );
}

#[test]
fn apply_chunk_deltas_rejects_duplicate_entities_before_mutating_world() {
    let mut app = app();
    let deltas = vec![
        delta_with_health(CHUNK, FIRST, 1),
        delta_with_health(OTHER_CHUNK, FIRST, 2),
    ];

    let err = apply_chunk_deltas(app.world_mut(), &deltas).unwrap_err();

    assert!(matches!(
        err,
        PersistenceError::DuplicateEntityDelta { entity } if entity == FIRST
    ));
    assert!(
        app.world()
            .resource::<StableEntityRegistry>()
            .entity(FIRST)
            .is_none()
    );
}

#[test]
fn apply_chunk_deltas_rejects_restore_delete_conflicts_before_mutating_world() {
    let mut app = app();
    let deltas = vec![
        delta_with_health(CHUNK, FIRST, 1),
        ChunkPersistentDelta {
            chunk: OTHER_CHUNK,
            entities: Vec::new(),
            deleted: vec![FIRST],
        },
    ];

    let err = apply_chunk_deltas(app.world_mut(), &deltas).unwrap_err();

    assert!(matches!(
        err,
        PersistenceError::ConflictingEntityDelta { entity } if entity == FIRST
    ));
    assert!(
        app.world()
            .resource::<StableEntityRegistry>()
            .entity(FIRST)
            .is_none()
    );
}

fn delta_with_health(chunk: ChunkId, entity: StableEntityId, current: u32) -> ChunkPersistentDelta {
    ChunkPersistentDelta {
        chunk,
        entities: vec![PersistentEntityDelta {
            entity,
            components: vec![value(&SavedHealth { current })],
            removed_components: Vec::new(),
        }],
        deleted: Vec::new(),
    }
}

fn value<T>(component: &T) -> PersistentComponentValue
where
    T: Serialize,
{
    PersistentComponentValue {
        type_name: "test.health.v1".into(),
        payload: serde_json::to_vec(component).unwrap(),
    }
}
