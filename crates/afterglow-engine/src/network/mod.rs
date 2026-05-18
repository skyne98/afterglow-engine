use bevy::prelude::*;

pub mod lightyear;
pub mod rewind;

pub use lightyear::{AfterglowLightyearConfig, AfterglowLightyearPlugin, LightyearRole};
pub use rewind::{
    ComponentHistory, HistoryEntry, RewindAppExt, RewindComponentRegistry, RewindDomainId,
    RewindHistoryBudget, RewindHistoryStore, RewindTick, RewindedEntity, ServerRewindPlugin,
    rewind_type_key,
};

pub struct AfterglowNetworkPlugin;

impl Plugin for AfterglowNetworkPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((AfterglowLightyearPlugin, ServerRewindPlugin));
    }
}
