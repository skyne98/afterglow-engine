#[cfg(not(target_arch = "wasm32"))]
use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

use crate::{
    core::identity::{
        ChunkId, ChunkMembership, Persistent, StableEntityId, StableEntityRegistry,
        maintain_stable_entity_registry,
    },
    setup::Rotates,
    world::lifecycle::{ChunkLifecycle, ChunkLifecycleRequests, ChunkLifecycleState},
};

pub const DEMO_CELL_CHUNK: ChunkId = ChunkId::from_raw(1);
pub const DEMO_CUBE_ID: StableEntityId = StableEntityId::from_raw(1_000);
pub const DEMO_LIGHT_ID: StableEntityId = StableEntityId::from_raw(1_001);
pub const DEMO_CAMERA_ID: StableEntityId = StableEntityId::from_raw(1_002);

#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct CellManifest {
    pub chunk: ChunkId,
    pub entities: Vec<CellEntityTemplate>,
}

#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct CellEntityTemplate {
    pub stable_id: StableEntityId,
    pub name: Option<String>,
    pub persistent: bool,
    pub transform: Transform,
    pub kind: CellEntityKind,
}

#[derive(Clone, Debug, PartialEq, Reflect)]
pub enum CellEntityKind {
    Empty,
    RotatingCube {
        size: f32,
        color: Srgba,
        rotation_speed: f32,
    },
    PointLight {
        intensity: f32,
        shadows_enabled: bool,
    },
    Camera3d,
}

#[derive(Resource, Clone, Debug, Default, PartialEq, Reflect)]
pub struct CellManifestRegistry {
    manifests: BTreeMap<ChunkId, CellManifest>,
}

#[derive(Resource, Clone, Debug, Default, Eq, PartialEq)]
pub struct CellLoadRequests {
    pending: BTreeSet<ChunkId>,
}

#[derive(Resource, Clone, Debug, Default, Eq, PartialEq, Reflect)]
pub struct CellLoadTracker {
    baseline_spawned: BTreeSet<ChunkId>,
}

#[derive(Resource, Clone, Debug, Default, Eq, PartialEq)]
pub struct CellLoadReport {
    pub requested_chunks: Vec<ChunkId>,
    pub spawned_chunks: Vec<ChunkId>,
    pub completed_chunks: Vec<ChunkId>,
    pub missing_chunks: Vec<ChunkId>,
    pub spawned_entities: usize,
    pub errors: Vec<CellLoadError>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CellLoadError {
    pub chunk: ChunkId,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CellLoadRequestError {
    #[error("cell load request has invalid chunk id")]
    InvalidChunkId,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CellManifestError {
    #[error("cell manifest has invalid chunk id")]
    InvalidChunkId,
    #[error("cell manifest contains invalid stable entity id")]
    InvalidStableEntityId,
    #[error("cell manifest contains duplicate stable entity id {0:?}")]
    DuplicateStableEntityId(StableEntityId),
}

impl CellManifestRegistry {
    pub fn with_demo_cell() -> Self {
        let mut registry = Self::default();
        registry
            .insert(demo_cell_manifest())
            .expect("built-in demo cell manifest is valid");
        registry
    }

    pub fn insert(&mut self, manifest: CellManifest) -> Result<(), CellManifestError> {
        validate_manifest(&manifest)?;
        self.manifests.insert(manifest.chunk, manifest);
        Ok(())
    }

    pub fn get(&self, chunk: ChunkId) -> Option<&CellManifest> {
        self.manifests.get(&chunk)
    }

    pub fn contains(&self, chunk: ChunkId) -> bool {
        self.manifests.contains_key(&chunk)
    }

    pub fn chunks(&self) -> impl Iterator<Item = ChunkId> + '_ {
        self.manifests.keys().copied()
    }
}

impl CellLoadRequests {
    pub fn with_demo_cell() -> Self {
        let mut requests = Self::default();
        requests
            .request_load(DEMO_CELL_CHUNK)
            .expect("built-in demo cell chunk is valid");
        requests
    }

    pub fn request_load(&mut self, chunk: ChunkId) -> Result<(), CellLoadRequestError> {
        if !chunk.is_valid() {
            return Err(CellLoadRequestError::InvalidChunkId);
        }
        self.pending.insert(chunk);
        Ok(())
    }

    pub fn pending(&self) -> &BTreeSet<ChunkId> {
        &self.pending
    }

    fn complete(&mut self, chunk: ChunkId) {
        self.pending.remove(&chunk);
    }
}

pub fn process_cell_load_requests(world: &mut World) {
    let pending = world.resource::<CellLoadRequests>().pending.clone();
    world.insert_resource(CellLoadReport::default());

    for chunk in pending {
        process_cell_load_request(world, chunk);
    }
}

pub fn demo_cell_manifest() -> CellManifest {
    CellManifest {
        chunk: DEMO_CELL_CHUNK,
        entities: vec![
            CellEntityTemplate {
                stable_id: DEMO_CUBE_ID,
                name: Some("Demo Cube".into()),
                persistent: true,
                transform: Transform::default(),
                kind: CellEntityKind::RotatingCube {
                    size: 1.0,
                    color: Srgba::new(0.2, 0.6, 1.0, 1.0),
                    rotation_speed: 0.5,
                },
            },
            CellEntityTemplate {
                stable_id: DEMO_LIGHT_ID,
                name: Some("Demo Light".into()),
                persistent: true,
                transform: Transform::from_xyz(3.0, 5.0, 2.0),
                kind: CellEntityKind::PointLight {
                    intensity: 2_000_000.0,
                    shadows_enabled: true,
                },
            },
            CellEntityTemplate {
                stable_id: DEMO_CAMERA_ID,
                name: Some("Demo Camera".into()),
                persistent: true,
                transform: Transform::from_xyz(-3.0, 2.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
                kind: CellEntityKind::Camera3d,
            },
        ],
    }
}

fn process_cell_load_request(world: &mut World, chunk: ChunkId) {
    if !world.resource::<CellManifestRegistry>().contains(chunk) {
        world.resource_mut::<CellLoadRequests>().complete(chunk);
        world
            .resource_mut::<CellLoadReport>()
            .missing_chunks
            .push(chunk);
        return;
    }

    let state = world.resource::<ChunkLifecycle>().state(chunk);
    match state {
        ChunkLifecycleState::Unloaded => {
            world
                .resource_mut::<CellLoadTracker>()
                .baseline_spawned
                .remove(&chunk);
            world
                .resource_mut::<ChunkLifecycleRequests>()
                .request_load(chunk)
                .expect("pending cell load chunk ids are valid");
            world
                .resource_mut::<CellLoadReport>()
                .requested_chunks
                .push(chunk);
        }
        ChunkLifecycleState::Loading => process_loading_cell(world, chunk),
        ChunkLifecycleState::Spawned
        | ChunkLifecycleState::GameplayActive
        | ChunkLifecycleState::Sleeping => {
            world.resource_mut::<CellLoadRequests>().complete(chunk);
            world
                .resource_mut::<CellLoadReport>()
                .completed_chunks
                .push(chunk);
        }
        ChunkLifecycleState::Unloading => {}
    }
}

fn process_loading_cell(world: &mut World, chunk: ChunkId) {
    let already_spawned = world
        .resource::<CellLoadTracker>()
        .baseline_spawned
        .contains(&chunk);

    if !already_spawned {
        let manifest = world
            .resource::<CellManifestRegistry>()
            .get(chunk)
            .expect("manifest presence was checked before loading")
            .clone();
        match spawn_cell_manifest(world, &manifest) {
            Ok(spawned) => {
                world
                    .resource_mut::<CellLoadTracker>()
                    .baseline_spawned
                    .insert(chunk);
                let mut report = world.resource_mut::<CellLoadReport>();
                report.spawned_chunks.push(chunk);
                report.spawned_entities += spawned;
            }
            Err(message) => {
                push_error(world, chunk, message);
                return;
            }
        }
    }

    world
        .resource_mut::<ChunkLifecycleRequests>()
        .request_spawned(chunk)
        .expect("pending cell load chunk ids are valid");
}

fn spawn_cell_manifest(world: &mut World, manifest: &CellManifest) -> Result<usize, String> {
    validate_manifest(manifest).map_err(|err| err.to_string())?;
    validate_manifest_spawn_dependencies(world, manifest)?;
    maintain_stable_entity_registry(world);
    validate_manifest_entity_conflicts(world, manifest)?;

    let mut spawned = 0;
    for template in &manifest.entities {
        let entity = get_or_spawn_manifest_entity(world, manifest.chunk, template, &mut spawned)?;
        apply_template_kind(world, entity, template)?;
    }
    maintain_stable_entity_registry(world);
    Ok(spawned)
}

fn validate_manifest_entity_conflicts(
    world: &World,
    manifest: &CellManifest,
) -> Result<(), String> {
    let registry = world.resource::<StableEntityRegistry>();
    for template in &manifest.entities {
        let Some(entity) = registry.entity(template.stable_id) else {
            continue;
        };
        let existing_chunk = world
            .get::<ChunkMembership>(entity)
            .map(|membership| membership.chunk);
        if existing_chunk.is_some_and(|existing_chunk| existing_chunk != manifest.chunk) {
            return Err(format!(
                "stable entity {:?} already belongs to chunk {:?}",
                template.stable_id, existing_chunk
            ));
        }
    }
    Ok(())
}

fn validate_manifest_spawn_dependencies(
    world: &World,
    manifest: &CellManifest,
) -> Result<(), String> {
    let needs_mesh_assets = manifest
        .entities
        .iter()
        .any(|template| matches!(template.kind, CellEntityKind::RotatingCube { .. }));
    if needs_mesh_assets && !world.contains_resource::<Assets<Mesh>>() {
        return Err("Assets<Mesh> resource is missing".into());
    }
    if needs_mesh_assets && !world.contains_resource::<Assets<StandardMaterial>>() {
        return Err("Assets<StandardMaterial> resource is missing".into());
    }
    Ok(())
}

fn get_or_spawn_manifest_entity(
    world: &mut World,
    chunk: ChunkId,
    template: &CellEntityTemplate,
    spawned: &mut usize,
) -> Result<Entity, String> {
    let existing = world
        .resource::<StableEntityRegistry>()
        .entity(template.stable_id);
    if let Some(entity) = existing {
        let existing_chunk = world
            .get::<ChunkMembership>(entity)
            .map(|membership| membership.chunk);
        if existing_chunk.is_some_and(|existing_chunk| existing_chunk != chunk) {
            return Err(format!(
                "stable entity {:?} already belongs to chunk {:?}",
                template.stable_id, existing_chunk
            ));
        }
        insert_manifest_base(world, entity, chunk, template);
        return Ok(entity);
    }

    let entity = world.spawn(template.stable_id).id();
    insert_manifest_base(world, entity, chunk, template);
    *spawned += 1;
    Ok(entity)
}

fn insert_manifest_base(
    world: &mut World,
    entity: Entity,
    chunk: ChunkId,
    template: &CellEntityTemplate,
) {
    let mut entity_mut = world.entity_mut(entity);
    entity_mut.insert((ChunkMembership::new(chunk), template.transform));
    if let Some(name) = &template.name {
        entity_mut.insert(Name::new(name.clone()));
    }
    if template.persistent {
        entity_mut.insert(Persistent);
    } else {
        entity_mut.remove::<Persistent>();
    }
}

fn apply_template_kind(
    world: &mut World,
    entity: Entity,
    template: &CellEntityTemplate,
) -> Result<(), String> {
    clear_builtin_template_components(world, entity);
    match &template.kind {
        CellEntityKind::Empty => {}
        CellEntityKind::RotatingCube {
            size,
            color,
            rotation_speed,
        } => {
            let mesh = world
                .get_resource_mut::<Assets<Mesh>>()
                .ok_or_else(|| "Assets<Mesh> resource is missing".to_string())?
                .add(Cuboid::from_size(Vec3::splat(*size)));
            let material = world
                .get_resource_mut::<Assets<StandardMaterial>>()
                .ok_or_else(|| "Assets<StandardMaterial> resource is missing".to_string())?
                .add(StandardMaterial {
                    base_color: Color::Srgba(*color),
                    ..default()
                });
            world.entity_mut(entity).insert((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Rotates {
                    speed: *rotation_speed,
                },
            ));
        }
        CellEntityKind::PointLight {
            intensity,
            shadows_enabled,
        } => {
            world.entity_mut(entity).insert(PointLight {
                intensity: *intensity,
                shadows_enabled: *shadows_enabled,
                ..default()
            });
        }
        CellEntityKind::Camera3d => {
            insert_camera(world, entity);
        }
    }
    Ok(())
}

fn clear_builtin_template_components(world: &mut World, entity: Entity) {
    let mut entity_mut = world.entity_mut(entity);
    entity_mut.remove::<(
        Mesh3d,
        MeshMaterial3d<StandardMaterial>,
        Rotates,
        PointLight,
    )>();
    entity_mut.remove::<(Camera3d, Msaa)>();
    #[cfg(not(target_arch = "wasm32"))]
    entity_mut.remove::<TemporalAntiAliasing>();
}

#[cfg(not(target_arch = "wasm32"))]
fn insert_camera(world: &mut World, entity: Entity) {
    world.entity_mut(entity).insert((
        Camera3d::default(),
        Msaa::Off,
        TemporalAntiAliasing::default(),
    ));
}

#[cfg(target_arch = "wasm32")]
fn insert_camera(world: &mut World, entity: Entity) {
    world
        .entity_mut(entity)
        .insert((Camera3d::default(), Msaa::Off));
}

fn validate_manifest(manifest: &CellManifest) -> Result<(), CellManifestError> {
    if !manifest.chunk.is_valid() {
        return Err(CellManifestError::InvalidChunkId);
    }

    let mut stable_ids = BTreeSet::new();
    for template in &manifest.entities {
        if !template.stable_id.is_valid() {
            return Err(CellManifestError::InvalidStableEntityId);
        }
        if !stable_ids.insert(template.stable_id) {
            return Err(CellManifestError::DuplicateStableEntityId(
                template.stable_id,
            ));
        }
    }
    Ok(())
}

fn push_error(world: &mut World, chunk: ChunkId, message: String) {
    world
        .resource_mut::<CellLoadReport>()
        .errors
        .push(CellLoadError { chunk, message });
}

#[cfg(test)]
mod tests;
