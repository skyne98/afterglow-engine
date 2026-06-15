use bevy::prelude::*;

use super::{AfterglowSessionState, SessionError, SessionEvent, SessionInfo, SessionMemberId};

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
    /// Last search results observed from `SessionEvent::SearchResults`.
    pub last_search_results: Vec<SessionInfo>,
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

/// Reads [`SessionEvent`]s and keeps [`SessionStatus`] and
/// [`AfterglowSessionState`] up to date.
///
/// Games can read either resource; `SessionStatus` is the recommended,
/// game-friendly view. `AfterglowSessionState` is the lower-level state used
/// by other systems (e.g. transport linking).
pub(crate) fn update_session_status(
    mut status: ResMut<SessionStatus>,
    mut state: ResMut<AfterglowSessionState>,
    mut events: MessageReader<SessionEvent>,
) {
    for event in events.read() {
        match event {
            SessionEvent::Created(info) => {
                status.info = Some(info.clone());
                status.state = SessionConnectionState::Joining;
                state.current_session = Some(info.id);
                state.local_member_id = info.owner;
                // The owner is already a member but is not delivered via a
                // separate MemberJoined event, so seed the list here.
                if !status.members.contains(&info.owner) {
                    status.members.push(info.owner);
                }
            }
            SessionEvent::Joined(info) => {
                status.info = Some(info.clone());
                status.state = SessionConnectionState::Joining;
                state.current_session = Some(info.id);
                // Do not set local_member_id from info.owner: for a joining
                // player info.owner is the session owner, not the local player.
                // The local member id comes from MemberJoined or from the
                // in-process handler that already wrote state.
                if !status.members.contains(&info.owner) {
                    status.members.push(info.owner);
                }
            }
            SessionEvent::MemberJoined { session, member } => {

                if !status.members.contains(member) {
                    status.members.push(*member);
                }
                // Remote joins learn their own member id from this event.
                if state.current_session == Some(*session)
                    && !state.local_member_id.is_valid()
                {
                    state.local_member_id = *member;
                }
            }
            SessionEvent::MemberLeft { member, .. } => {
                status.members.retain(|m| m != member);
            }
            SessionEvent::Left { .. } | SessionEvent::SessionEnded(_) => {
                *status = SessionStatus::default();
                state.current_session = None;
                state.current_backend = None;
                state.identity = None;
                state.local_member_id = SessionMemberId::INVALID;
            }
            SessionEvent::Error(err) => {
                status.state = SessionConnectionState::Error(*err);
            }
            // Search results do not change local session status.
            SessionEvent::SearchResults(results) => {
                status.last_search_results = results.clone();
            }
        }
    }
}
