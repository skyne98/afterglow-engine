use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "lightyear")]
pub mod connection;
pub mod context;
pub mod interpolation;
pub mod lightyear;

/// Player identifier = netcode `client_id` = authenticated identity.
pub type PlayerId = u64;

#[cfg(feature = "lightyear")]
pub use connection::{
    AfterglowConnectionPlugin, AuthResponse, ChallengeMessage, ConnectionConfig, ConnectionEvent,
    ConnectionEventKind, ControlledEntityPlugin, LocalIdentity, LocalPlayerId, MemberLinkMap,
    NetcodeConfig, PlayerOwned, ServerAddr, ServerListenAddr,
};
pub use context::AfterglowNetworkContext;
pub use interpolation::{NetworkTransformInterpolationBuffer, NetworkTransformSample};
#[cfg(feature = "lightyear")]
pub use lightyear::request_rollback_check_on_confirmed_tick_changed;
pub use lightyear::{
    AfterglowLightyearConfig, AfterglowLightyearPlugin, LightyearLinkConditioner, LightyearRole,
    register_afterglow_lightyear_protocol,
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
