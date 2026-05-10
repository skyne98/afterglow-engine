pub mod identity;

use bevy::prelude::*;
use identity::{StableEntityRegistry, StableIdAllocator, maintain_stable_entity_registry};

pub struct AfterglowCorePlugin;

impl Plugin for AfterglowCorePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StableIdAllocator>()
            .init_resource::<StableEntityRegistry>()
            .add_systems(PostUpdate, maintain_stable_entity_registry);
    }
}
