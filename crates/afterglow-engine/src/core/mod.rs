pub mod identity;
pub mod schedule;

use bevy::prelude::*;
use identity::{StableEntityRegistry, StableIdAllocator, maintain_stable_entity_registry};
use schedule::configure_engine_sets;

pub struct AfterglowCorePlugin;

impl Plugin for AfterglowCorePlugin {
    fn build(&self, app: &mut App) {
        configure_engine_sets(app);
        app.init_resource::<StableIdAllocator>()
            .init_resource::<StableEntityRegistry>()
            .add_systems(PostUpdate, maintain_stable_entity_registry);
    }
}
