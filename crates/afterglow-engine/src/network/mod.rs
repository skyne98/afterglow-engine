use bevy::prelude::*;
use serde::{Deserialize, Serialize};

pub mod context;
pub mod controlled;
pub mod interpolation;
pub mod lightyear;
pub mod session;

pub use context::{AfterglowConnectionStatus, AfterglowNetworkContext};
pub use controlled::{
    ControlledEntityPlugin, MemberLinkMap, OwnershipSource, PlayerOwned,
    bind_controlled_entities, update_member_link_map,
};
pub use interpolation::{NetworkTransformInterpolationBuffer, NetworkTransformSample};
pub use lightyear::{
    AfterglowLightyearConfig, AfterglowLightyearPlugin, LightyearLinkConditioner, LightyearRole,
    register_afterglow_lightyear_protocol,
};
#[cfg(feature = "lightyear")]
pub use lightyear::{
    AfterglowSessionLightyearBridgePlugin, NetcodeClientParams, NetcodeServerParams,
    PendingNetcodeStartup, SessionLightyearLinks,
};
pub use session::{
    AfterglowSessionPlugin, AfterglowSessionSet, AfterglowSessionState, IdentityError,
    NativeIdentityProof, PlayerIdentity, SessionBackend, SessionCode, SessionConfig, SessionError,
    SessionEvent, SessionId, SessionIdentityNonce, SessionInfo, SessionLeaveReason,
    SessionMemberId, SessionRequest, SessionSearch, SessionTransport, SessionVisibility,
};

/// Simple tick counter for fixed-step systems that need a shared authoritative
/// tick. This is not a rewind system; it is just a shared `u32`.
#[derive(Resource, Clone, Copy, Debug, Default, Eq, PartialEq, Reflect, Serialize, Deserialize)]
pub struct HistoryTick(pub u32);

pub struct AfterglowNetworkPlugin;

impl Plugin for AfterglowNetworkPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((AfterglowLightyearPlugin, AfterglowSessionPlugin));
    }
}
