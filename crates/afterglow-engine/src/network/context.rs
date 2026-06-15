use bevy::prelude::*;

use super::{
    lightyear::{AfterglowLightyearConfig, LightyearRole},
    session::{
        AfterglowSessionState, SessionConnectionState, SessionId, SessionMemberId, SessionStatus,
    },
};

#[derive(Resource, Clone, Debug, Default, Eq, PartialEq)]
pub struct AfterglowNetworkContext {
    status: AfterglowConnectionStatus,
}

impl AfterglowNetworkContext {
    pub fn from_status(status: AfterglowConnectionStatus) -> Self {
        Self { status }
    }

    pub fn get_connection_status(&self) -> &AfterglowConnectionStatus {
        &self.status
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AfterglowConnectionStatus {
    pub role: LightyearRole,
    pub session_state: SessionConnectionState,
    pub session_id: Option<SessionId>,
    pub local_member_id: SessionMemberId,
    pub member_count: usize,
}

impl AfterglowConnectionStatus {
    pub fn runs_authority(&self) -> bool {
        matches!(self.role, LightyearRole::Host | LightyearRole::Server)
    }

    pub fn runs_client_prediction(&self) -> bool {
        matches!(self.role, LightyearRole::Host | LightyearRole::Client)
    }

    pub fn is_host(&self) -> bool {
        self.role == LightyearRole::Host
    }

    pub fn is_client_only(&self) -> bool {
        self.role == LightyearRole::Client
    }

    pub fn is_server_only(&self) -> bool {
        self.role == LightyearRole::Server
    }

    pub fn is_in_session(&self) -> bool {
        self.session_id.is_some()
    }

    pub fn local_member_owner(&self) -> Option<String> {
        self.local_member_id
            .is_valid()
            .then(|| self.local_member_id.as_raw().to_string())
    }

    pub fn owns_member(&self, member: SessionMemberId) -> bool {
        self.local_member_id.is_valid() && self.local_member_id == member
    }
}

pub(crate) fn update_network_context(
    mut context: ResMut<AfterglowNetworkContext>,
    lightyear: Option<Res<AfterglowLightyearConfig>>,
    session_state: Option<Res<AfterglowSessionState>>,
    session_status: Option<Res<SessionStatus>>,
) {
    let role = lightyear.as_deref().map(|cfg| cfg.role).unwrap_or_default();
    let session_id = session_state
        .as_deref()
        .and_then(|state| state.current_session);
    let local_member_id = session_state
        .as_deref()
        .map(|state| state.local_member_id)
        .unwrap_or(SessionMemberId::INVALID);
    let (session_connection_state, member_count) = session_status
        .as_deref()
        .map(|status| (status.state, status.member_count()))
        .unwrap_or_default();

    context.status = AfterglowConnectionStatus {
        role,
        session_state: session_connection_state,
        session_id,
        local_member_id,
        member_count,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reports_authority_and_prediction_roles() {
        let host = AfterglowConnectionStatus {
            role: LightyearRole::Host,
            ..Default::default()
        };
        assert!(host.runs_authority());
        assert!(host.runs_client_prediction());

        let client = AfterglowConnectionStatus {
            role: LightyearRole::Client,
            ..Default::default()
        };
        assert!(!client.runs_authority());
        assert!(client.runs_client_prediction());
    }

    #[test]
    fn local_member_owner_is_stable_string_key() {
        let status = AfterglowConnectionStatus {
            local_member_id: SessionMemberId::new(42),
            ..Default::default()
        };
        assert_eq!(status.local_member_owner().as_deref(), Some("42"));
        assert!(status.owns_member(SessionMemberId::new(42)));
        assert!(!status.owns_member(SessionMemberId::new(7)));
    }
}
