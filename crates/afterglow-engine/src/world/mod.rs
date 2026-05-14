pub mod cell;
pub mod lifecycle;

use crate::persistence::{PersistenceRegistry, PersistentWorldDeltas};
use bevy::prelude::*;
use cell::{
    CellLoadReport, CellLoadRequests, CellLoadTracker, CellManifestRegistry,
    process_cell_load_requests,
};
use lifecycle::{
    ChunkLifecycle, ChunkLifecycleConfig, ChunkLifecycleReport, ChunkLifecycleRequests,
    process_chunk_lifecycle_requests,
};

pub struct AfterglowWorldPlugin;

impl Plugin for AfterglowWorldPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CellManifestRegistry>()
            .init_resource::<CellLoadRequests>()
            .init_resource::<CellLoadTracker>()
            .init_resource::<CellLoadReport>()
            .init_resource::<ChunkLifecycle>()
            .init_resource::<ChunkLifecycleConfig>()
            .init_resource::<ChunkLifecycleRequests>()
            .init_resource::<ChunkLifecycleReport>()
            .init_resource::<PersistentWorldDeltas>()
            .init_resource::<PersistenceRegistry>()
            .add_systems(
                Update,
                process_cell_load_requests
                    .in_set(crate::core::schedule::AfterglowSet::ApplyGameplay),
            )
            .add_systems(
                Update,
                process_chunk_lifecycle_requests
                    .in_set(crate::core::schedule::AfterglowSet::PreparePersistence),
            );
    }
}
