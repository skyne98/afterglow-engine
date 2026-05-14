pub mod chunk;
pub mod lifecycle;

use crate::persistence::{PersistenceRegistry, PersistentWorldDeltas};
use bevy::prelude::*;
use chunk::{DemoCellState, load_demo_cell};
use lifecycle::{
    ChunkLifecycle, ChunkLifecycleConfig, ChunkLifecycleReport, ChunkLifecycleRequests,
    process_chunk_lifecycle_requests,
};

pub struct AfterglowWorldPlugin;

impl Plugin for AfterglowWorldPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DemoCellState>()
            .init_resource::<ChunkLifecycle>()
            .init_resource::<ChunkLifecycleConfig>()
            .init_resource::<ChunkLifecycleRequests>()
            .init_resource::<ChunkLifecycleReport>()
            .init_resource::<PersistentWorldDeltas>()
            .init_resource::<PersistenceRegistry>()
            .add_systems(
                Update,
                process_chunk_lifecycle_requests
                    .in_set(crate::core::schedule::AfterglowSet::PreparePersistence),
            )
            .add_systems(Startup, load_demo_cell);
    }
}
