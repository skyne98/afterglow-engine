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
        let install_demo_cell = !app.world().contains_resource::<CellManifestRegistry>();
        if install_demo_cell {
            app.insert_resource(CellManifestRegistry::with_demo_cell());
        }
        if !app.world().contains_resource::<CellLoadRequests>() {
            let requests = if install_demo_cell {
                CellLoadRequests::with_demo_cell()
            } else {
                CellLoadRequests::default()
            };
            app.insert_resource(requests);
        }

        app.init_resource::<CellLoadTracker>()
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
