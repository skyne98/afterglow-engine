use bevy::prelude::*;

use super::{SessionError, SessionEvent, SessionInfo, SessionMemberId};

/// High-level snapshot of the local player's session membership.
///
/// `AfterglowSessionState` tracks the raw ids assigned by the provider.
/// `SessionStatus` keeps a read-only, game-friendly snapshot derived from
/// `SessionEvent`s: the current `SessionInfo`, member list, and connection
/// state.
#[derive(Resource, Debug, Default)]
pub struct SessionStatus {
    /// Last known `SessionInfo` from `Created` or `Joined`.
    pub info: Option<SessionInfo>,
    /// Current member ids observed from `MemberJoined` / `MemberLeft`.
    pub members: Vec<SessionMemberId>,
    /// Current connection state.
    pub state: SessionConnectionState,
}

impl SessionStatus {
    /// Whether the local player has an active session.
    pub fn is_in_session(&self) -> bool {
        self.info.is_some()
    }

    /// Number of members currently in the session.
    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    /// Whether the session is in an error state.
    pub fn is_error(&self) -> Option<&SessionError> {
        match &self.state {
            SessionConnectionState::Error(err) => Some(err),
            _ => None,
        }
    }
}

/// Coarse state of the local session connection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SessionConnectionState {
    /// No current session.
    #[default]
    Idle,
    /// Join/create request accepted; waiting for transport link.
    Joining,
    /// Gameplay transport link established.
    Connected,
    /// A session request failed.
    Error(SessionError),
}

/// Reads [`SessionEvent`]s and keeps [`SessionStatus`] up to date.
pub(crate) fn update_session_status(
    mut status: ResMut<SessionStatus>,
    mut events: MessageReader<SessionEvent>,
) {
    for event in events.read() {
        match event {
            SessionEvent::Created(info) | SessionEvent::Joined(info) => {
                status.info = Some(info.clone());
                status.state = SessionConnectionState::Joining;
                // The owner is already a member but is not delivered via a
                // separate MemberJoined event, so seed the list here.
                if !status.members.contains(&info.owner) {
                    status.members.push(info.owner);
                }
            }
            SessionEvent::MemberJoined { member, .. } => {
                if !status.members.contains(member) {
                    status.members.push(*member);
                }
            }
            SessionEvent::MemberLeft { member, .. } => {
                status.members.retain(|m| m != member);
            }
            SessionEvent::Left { .. } | SessionEvent::SessionEnded(_) => {
                *status = SessionStatus::default();
            }
            SessionEvent::Error(err) => {
                status.state = SessionConnectionState::Error(*err);
            }
            // Search results do not change local session status.
            SessionEvent::SearchResults(_) => {}
        }
    }
}
