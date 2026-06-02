use bevy::prelude::*;
use serde::{Deserialize, Serialize};

pub mod interpolation;
pub mod lightyear;

pub use interpolation::{NetworkTransformInterpolationBuffer, NetworkTransformSample};
pub use lightyear::{
    AfterglowLightyearConfig, AfterglowLightyearPlugin, LightyearLinkConditioner, LightyearRole,
};

/// Simple tick counter for fixed-step systems that need a shared authoritative
/// tick. This is not a rewind system; it is just a shared `u32`.
#[derive(Resource, Clone, Copy, Debug, Default, Eq, PartialEq, Reflect, Serialize, Deserialize)]
pub struct HistoryTick(pub u32);

pub struct AfterglowNetworkPlugin;

impl Plugin for AfterglowNetworkPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(AfterglowLightyearPlugin);
    }
}
