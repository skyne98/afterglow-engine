//! High-level, minimal session API.
//!
//! [`AfterglowSessionExt`] adds a tiny fluent surface to [`bevy::prelude::App`]
//! so games can host, join, leave, and query sessions without writing raw
//! [`SessionRequest`](crate::network::session::SessionRequest) messages.

use bevy::prelude::*;
use std::net::SocketAddr;

use crate::network::session::{
    AfterglowSessionState, NonSteamSessionProvider, PlayerIdentity, ProviderEndpoint,
    SessionBackend, SessionCode, SessionConfig, SessionRequest, SessionSearch, SessionStatus,
};

/// Extension trait for [`App`] that exposes the Afterglow session API.
pub trait AfterglowSessionExt {
    /// Returns a session handle for the current app.
    fn session(&mut self) -> SessionHandle<'_>;
}

impl AfterglowSessionExt for App {
    fn session(&mut self) -> SessionHandle<'_> {
        SessionHandle { app: self }
    }
}

/// Fluent handle for issuing session operations against an [`App`].
///
/// Created via [`AfterglowSessionExt::session`].
pub struct SessionHandle<'a> {
    app: &'a mut App,
}

impl SessionHandle<'_> {
    /// Host a session using the given config and identity.
    pub fn host(&mut self, config: SessionConfig, identity: PlayerIdentity) {
        self.app
            .world_mut()
            .write_message(SessionRequest::Create(config, identity));
    }

    /// Host a NonSteam listen-server on the given address.
    pub fn host_with_endpoint(
        &mut self,
        config: SessionConfig,
        identity: PlayerIdentity,
        provider: SocketAddr,
    ) -> Result<(), std::io::Error> {
        self.app
            .world_mut()
            .insert_resource(NonSteamSessionProvider::new(provider)?);
        self.host(config, identity);
        Ok(())
    }

    /// Join a NonSteam session by its short code and provider address.
    pub fn join_non_steam(
        &mut self,
        code: SessionCode,
        provider: SocketAddr,
        identity: PlayerIdentity,
    ) {
        self.app.world_mut().write_message(SessionRequest::JoinByCode {
            backend: SessionBackend::NonSteam,
            provider: ProviderEndpoint::Udp(provider),
            code,
            identity,
        });
    }

    /// Join a Steam lobby by its short code.
    pub fn join_steam(&mut self, code: SessionCode, identity: PlayerIdentity) {
        self.app.world_mut().write_message(SessionRequest::JoinByCode {
            backend: SessionBackend::Steam,
            provider: ProviderEndpoint::Steam,
            code,
            identity,
        });
    }

    /// Join an in-process session by its short code.
    pub fn join_local(&mut self, code: SessionCode, identity: PlayerIdentity) {
        self.app.world_mut().write_message(SessionRequest::JoinByCode {
            backend: SessionBackend::NonSteam,
            provider: ProviderEndpoint::InProcess,
            code,
            identity,
        });
    }

    /// Leave the current session.
    pub fn leave(&mut self) {
        self.app.world_mut().write_message(SessionRequest::Leave);
    }

    /// Search a NonSteam provider for sessions matching the given metadata.
    pub fn search_non_steam(
        &mut self,
        provider: SocketAddr,
        metadata: std::collections::HashMap<String, String>,
    ) {
        self.app.world_mut().write_message(SessionRequest::Search(SessionSearch {
            backend: SessionBackend::NonSteam,
            provider: ProviderEndpoint::Udp(provider),
            metadata,
            require_open_slot: true,
            max_results: 16,
        }));
    }

    /// Current session status snapshot.
    pub fn status(&self) -> &SessionStatus {
        self.app.world().resource::<SessionStatus>()
    }

    /// Whether the local player is currently in a session.
    pub fn is_in_session(&self) -> bool {
        self.status().is_in_session()
    }

    /// Low-level session state resource.
    pub fn state(&self) -> &AfterglowSessionState {
        self.app.world().resource::<AfterglowSessionState>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::session::{
        tests::test_app, PlayerIdentity, SessionBackend, SessionIdentityNonce, SessionTransport,
    };

    fn identity_fixture() -> PlayerIdentity {
        PlayerIdentity::Steam {
            steam_id: 1,
            ticket: vec![],
        }
    }

    #[test]
    fn api_host_writes_create_request() {
        let mut app = test_app();
        app.session().host(
            SessionConfig {
                backend: SessionBackend::NonSteam,
                transport: SessionTransport::Local,
                ..Default::default()
            },
            identity_fixture(),
        );
        app.update();
        assert!(app.world().resource::<SessionStatus>().is_in_session());
    }

    #[test]
    fn api_host_with_endpoint_inserts_provider() {
        let mut app = test_app();
        app.session()
            .host_with_endpoint(
                SessionConfig {
                    backend: SessionBackend::NonSteam,
                    transport: SessionTransport::DirectUdp {
                        host: "127.0.0.1:8822".into(),
                    },
                    ..Default::default()
                },
                identity_fixture(),
                "127.0.0.1:8823".parse().unwrap(),
            )
            .unwrap();
        assert!(app.world().contains_resource::<NonSteamSessionProvider>());
    }

    #[test]
    fn api_join_local_round_trip() {
        let mut app = test_app();
        app.world_mut().insert_resource(SessionIdentityNonce([0u8; 32]));

        app.session().host(
            SessionConfig {
                backend: SessionBackend::NonSteam,
                transport: SessionTransport::Local,
                name: "api-test".into(),
                ..Default::default()
            },
            identity_fixture(),
        );
        app.update();

        let code = app
            .world()
            .resource::<SessionStatus>()
            .info
            .as_ref()
            .unwrap()
            .code
            .clone();

        app.session().join_local(code, identity_fixture());
        for _ in 0..5 {
            app.update();
        }
        assert!(app.world().resource::<SessionStatus>().is_in_session());
    }
}
