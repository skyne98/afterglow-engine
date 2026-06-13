use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub mod code;
pub(crate) mod entry;
pub mod identity;

pub use code::{
    SESSION_CODE_ALPHABET, SESSION_CODE_CHAR_LEN, SESSION_CODE_GROUP_LEN, SESSION_CODE_GROUPS,
    SessionCode,
};
pub use identity::{IdentityError, NativeIdentityProof, PlayerIdentity, SessionIdentityNonce};

pub(crate) mod non_steam;

// ---------------------------------------------------------------------------
// ID Types
// ---------------------------------------------------------------------------

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    Serialize,
    Deserialize,
    Reflect,
)]
pub struct SessionId(pub u128);

impl SessionId {
    pub const INVALID: Self = Self(0);

    pub const fn new(raw: u128) -> Self {
        Self(raw)
    }

    pub const fn from_raw(raw: u128) -> Self {
        Self(raw)
    }

    pub const fn as_raw(self) -> u128 {
        self.0
    }

    pub const fn is_valid(self) -> bool {
        self.0 != 0
    }
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    Serialize,
    Deserialize,
    Reflect,
)]
pub struct SessionMemberId(pub u128);

impl SessionMemberId {
    pub const INVALID: Self = Self(0);

    pub const fn new(raw: u128) -> Self {
        Self(raw)
    }

    pub const fn from_raw(raw: u128) -> Self {
        Self(raw)
    }

    pub const fn as_raw(self) -> u128 {
        self.0
    }

    pub const fn is_valid(self) -> bool {
        self.0 != 0
    }
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum SessionBackend {
    #[default]
    NonSteam,
    Steam,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum SessionVisibility {
    #[default]
    Private,
    FriendsOnly,
    Public,
    Invisible,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum SessionTransport {
    #[default]
    Local,
    DirectUdp { host: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SessionLeaveReason {
    Left,
    Disconnected,
    Kicked,
    Banned,
    HostEnded,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SessionError {
    AlreadyInSession,
    NotInSession,
    SessionNotFound,
    SessionFull,
    InvalidConfig,
    PermissionDenied,
    BackendUnavailable,
}

// ---------------------------------------------------------------------------
// Config / Info
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionConfig {
    pub name: String,
    pub backend: SessionBackend,
    pub max_members: u32,
    pub visibility: SessionVisibility,
    pub metadata: HashMap<String, String>,
    pub transport: SessionTransport,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            backend: SessionBackend::NonSteam,
            max_members: 4,
            visibility: SessionVisibility::default(),
            metadata: HashMap::new(),
            transport: SessionTransport::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionSearch {
    pub backend: SessionBackend,
    pub metadata: HashMap<String, String>,
    pub require_open_slot: bool,
    pub max_results: u32,
}

impl Default for SessionSearch {
    fn default() -> Self {
        Self {
            backend: SessionBackend::NonSteam,
            metadata: HashMap::new(),
            require_open_slot: false,
            max_results: 16,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: SessionId,
    pub code: SessionCode,
    pub backend: SessionBackend,
    pub name: String,
    pub owner: SessionMemberId,
    pub owner_identity: PlayerIdentity,
    pub member_count: u32,
    pub max_members: u32,
    pub visibility: SessionVisibility,
    pub metadata: HashMap<String, String>,
    pub transport: SessionTransport,
}

// ---------------------------------------------------------------------------
// Messages & Resources
// ---------------------------------------------------------------------------

/// Session lifecycle system ordering.
///
/// Providers that turn [`SessionRequest`] messages into [`SessionEvent`]
/// messages should run in [`AfterglowSessionSet::ProcessRequests`]. Systems
/// that consume session outcomes to update network/runtime state should run in
/// [`AfterglowSessionSet::ApplyEffects`].
#[derive(SystemSet, Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AfterglowSessionSet {
    ProcessRequests,
    ApplyEffects,
}

/// Request sent to the session/matchmaking layer. This is a Bevy `Message`.
///
/// Variants:
/// - `Create(SessionConfig, PlayerIdentity)` — create a new session, proving
///   player identity.
/// - `Search(SessionSearch)` — search for existing sessions matching filters.
///   Search does not require identity.
/// - `Join { backend, session, identity }` — join a specific session on the
///   given backend by its internal [`SessionId`].
/// - `JoinByCode { backend, code, identity }` — join a session by its
///   player-facing [`SessionCode`].
/// - `Leave` — leave the current session.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum SessionRequest {
    Create(SessionConfig, PlayerIdentity),
    Search(SessionSearch),
    Join {
        backend: SessionBackend,
        session: SessionId,
        identity: PlayerIdentity,
    },
    JoinByCode {
        backend: SessionBackend,
        code: SessionCode,
        identity: PlayerIdentity,
    },
    Leave,
}

/// Carries session lifecycle outcomes. This is a Bevy `Message`.
///
/// Emitted by the session layer after processing a [`SessionRequest`].
#[derive(Message, Clone, Debug, PartialEq)]
pub enum SessionEvent {
    Created(SessionInfo),
    SearchResults(Vec<SessionInfo>),
    Joined(SessionInfo),
    Left {
        session: SessionId,
        reason: SessionLeaveReason,
    },
    MemberJoined {
        session: SessionId,
        member: SessionMemberId,
    },
    MemberLeft {
        session: SessionId,
        member: SessionMemberId,
        reason: SessionLeaveReason,
    },
    SessionEnded(SessionId),
    Error(SessionError),
}

/// Tracks the local member's session state.
#[derive(Resource, Debug)]
pub struct AfterglowSessionState {
    pub local_member_id: SessionMemberId,
    pub identity: Option<PlayerIdentity>,
    pub current_session: Option<SessionId>,
    pub current_backend: Option<SessionBackend>,
}

impl Default for AfterglowSessionState {
    fn default() -> Self {
        Self {
            local_member_id: SessionMemberId::INVALID,
            identity: None,
            current_session: None,
            current_backend: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests (separate file to keep source files under 500 LOC)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

pub struct AfterglowSessionPlugin;

impl Plugin for AfterglowSessionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AfterglowSessionState>()
            .init_resource::<non_steam::NonSteamSessionCatalog>()
            .init_resource::<SessionIdentityNonce>()
            .add_message::<SessionRequest>()
            .add_message::<SessionEvent>()
            .configure_sets(
                PreUpdate,
                (
                    AfterglowSessionSet::ProcessRequests,
                    AfterglowSessionSet::ApplyEffects,
                )
                    .chain(),
            )
            .add_systems(
                PreUpdate,
                non_steam::process_non_steam_session_requests
                    .in_set(AfterglowSessionSet::ProcessRequests),
            );
    }
}
