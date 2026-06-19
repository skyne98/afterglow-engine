pub mod identity;
pub mod schedule;

use bevy::{prelude::*, time::Fixed};
use identity::{
    AutoStableEntityId, RuntimeOnly, StableEntityId, StableIdAllocator,
    assign_auto_stable_entity_ids,
};
use schedule::configure_engine_sets;

pub struct AfterglowCorePlugin;

impl Plugin for AfterglowCorePlugin {
    fn build(&self, app: &mut App) {
        configure_engine_sets(app);
        app.insert_resource(Time::<Fixed>::from_hz(60.0))
            .init_resource::<StableIdAllocator>()
            .register_type::<StableEntityId>()
            .register_type::<AutoStableEntityId>()
            .register_type::<RuntimeOnly>()
            .add_systems(PreUpdate, assign_auto_stable_entity_ids);
    }
}
