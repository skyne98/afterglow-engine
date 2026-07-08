use bevy::prelude::*;

use super::{PlayerId, lightyear::LightyearRole};

/// Simplified network context that provides role and local player info.
///
/// This replaces the old version that depended on session types.
/// The engine keeps this resource updated based on the active connection.
#[derive(Resource, Clone, Debug, Default, Eq, PartialEq)]
pub struct AfterglowNetworkContext {
    pub role: LightyearRole,
    pub local_player_id: PlayerId,
}

impl AfterglowNetworkContext {
    pub fn new(role: LightyearRole, local_player_id: PlayerId) -> Self {
        Self {
            role,
            local_player_id,
        }
    }

    pub fn get_connection_status(&self) -> &Self {
        self
    }

    pub fn runs_authority(&self) -> bool {
        self.role == LightyearRole::Server
    }

    pub fn runs_client_prediction(&self) -> bool {
        self.role == LightyearRole::Client
    }

    pub fn is_client_only(&self) -> bool {
        self.role == LightyearRole::Client
    }

    pub fn is_server_only(&self) -> bool {
        self.role == LightyearRole::Server
    }

    /// Returns the local player's id as a string, for comparison with
    /// `PlayerBox.owner`.
    pub fn local_member_owner(&self) -> Option<String> {
        if self.local_player_id != 0 {
            Some(self.local_player_id.to_string())
        } else {
            None
        }
    }

    pub fn get_local_player_id(&self) -> PlayerId {
        self.local_player_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reports_authority_and_prediction_roles() {
        let server = AfterglowNetworkContext {
            role: LightyearRole::Server,
            local_player_id: 42,
        };
        assert!(server.runs_authority());
        assert!(!server.runs_client_prediction());

        let client = AfterglowNetworkContext {
            role: LightyearRole::Client,
            local_player_id: 42,
        };
        assert!(!client.runs_authority());
        assert!(client.runs_client_prediction());
    }

    #[test]
    fn local_member_owner_is_stable_string_key() {
        let ctx = AfterglowNetworkContext {
            role: LightyearRole::Client,
            local_player_id: 42,
        };
        assert_eq!(ctx.local_member_owner().as_deref(), Some("42"));
    }
}
