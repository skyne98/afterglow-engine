#[cfg(not(target_arch = "wasm32"))]
use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::prelude::*;

use crate::{
    core::identity::{ChunkId, ChunkMembership, Persistent, StableEntityId},
    setup::Rotates,
};

pub const DEMO_CELL_CHUNK: ChunkId = ChunkId::from_raw(1);
const DEMO_CUBE_ID: StableEntityId = StableEntityId::from_raw(1_000);
const DEMO_LIGHT_ID: StableEntityId = StableEntityId::from_raw(1_001);
const DEMO_CAMERA_ID: StableEntityId = StableEntityId::from_raw(1_002);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub enum ChunkLoadState {
    Unloaded,
    Loaded,
}

#[derive(Resource, Debug, Reflect)]
pub struct DemoCellState {
    pub chunk: ChunkId,
    pub load_state: ChunkLoadState,
}

impl Default for DemoCellState {
    fn default() -> Self {
        Self {
            chunk: DEMO_CELL_CHUNK,
            load_state: ChunkLoadState::Unloaded,
        }
    }
}

pub fn load_demo_cell(
    mut commands: Commands,
    mut state: ResMut<DemoCellState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if state.load_state == ChunkLoadState::Loaded {
        return;
    }

    let chunk = state.chunk;
    let membership = ChunkMembership::new(chunk);

    commands.spawn((
        StableEntityId(DEMO_CUBE_ID.as_raw()),
        membership,
        Persistent,
        Mesh3d(meshes.add(Cuboid::from_size(Vec3::splat(1.0)))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.2, 0.6, 1.0),
            ..default()
        })),
        Rotates { speed: 0.5 },
    ));

    commands.spawn((
        StableEntityId(DEMO_LIGHT_ID.as_raw()),
        membership,
        Persistent,
        PointLight {
            intensity: 2_000_000.,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(3.0, 5.0, 2.0),
    ));

    let camera_transform = Transform::from_xyz(-3.0, 2.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y);

    #[cfg(not(target_arch = "wasm32"))]
    commands.spawn((
        StableEntityId(DEMO_CAMERA_ID.as_raw()),
        membership,
        Persistent,
        Camera3d::default(),
        Msaa::Off,
        TemporalAntiAliasing::default(),
        camera_transform,
    ));

    #[cfg(target_arch = "wasm32")]
    commands.spawn((
        StableEntityId(DEMO_CAMERA_ID.as_raw()),
        membership,
        Persistent,
        Camera3d::default(),
        Msaa::Off,
        camera_transform,
    ));

    state.load_state = ChunkLoadState::Loaded;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{core::AfterglowCorePlugin, world::AfterglowWorldPlugin};

    #[test]
    fn demo_cell_spawns_stable_chunk_members() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            AfterglowCorePlugin,
            AfterglowWorldPlugin,
        ))
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();

        app.update();
        app.update();

        let registry = app
            .world()
            .resource::<crate::core::identity::StableEntityRegistry>();
        let chunk_entities = registry.chunk_entities(DEMO_CELL_CHUNK);
        assert_eq!(chunk_entities.len(), 3);
        assert!(registry.entity(DEMO_CUBE_ID).is_some());
        assert!(registry.entity(DEMO_LIGHT_ID).is_some());
        assert!(registry.entity(DEMO_CAMERA_ID).is_some());
    }

    #[test]
    fn demo_cell_load_is_idempotent() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            AfterglowCorePlugin,
            AfterglowWorldPlugin,
        ))
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();

        app.update();
        app.update();
        app.update();

        let registry = app
            .world()
            .resource::<crate::core::identity::StableEntityRegistry>();
        assert_eq!(registry.chunk_entities(DEMO_CELL_CHUNK).len(), 3);
    }
}
