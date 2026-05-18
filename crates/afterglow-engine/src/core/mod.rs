pub mod identity;
pub mod schedule;

use bevy::prelude::*;
use identity::{
    ChunkId, ChunkMembership, Persistent, Replicated, RuntimeOnly, StableEntityId,
    StableEntityRegistry, StableIdAllocator, maintain_stable_entity_registry,
};
use schedule::configure_engine_sets;

pub struct AfterglowCorePlugin;

impl Plugin for AfterglowCorePlugin {
    fn build(&self, app: &mut App) {
        configure_engine_sets(app);
        app.init_resource::<StableIdAllocator>()
            .init_resource::<StableEntityRegistry>()
            .register_type::<StableEntityId>()
            .register_type::<ChunkId>()
            .register_type::<ChunkMembership>()
            .register_type::<Persistent>()
            .register_type::<Replicated>()
            .register_type::<RuntimeOnly>()
            .add_systems(PostUpdate, maintain_stable_entity_registry);
    }
}
