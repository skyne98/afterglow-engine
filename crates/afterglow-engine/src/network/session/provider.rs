use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

/// Where to send session control-plane requests.
///
/// The data plane (actual gameplay traffic) is separate; this endpoint only
/// identifies the listener that handles `SessionRequest` / `SessionEvent`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ProviderEndpoint {
    /// In-process catalog. Used for tests, title screen, and local
    /// split-screen logic where the provider and client live in the same app.
    InProcess,
    /// NonSteam UDP/netcode session listener, usually the host's socket
    /// address. The joiner must obtain this address out of band (Discord,
    /// LAN discovery, etc.).
    Udp(SocketAddr),
    /// Steamworks resolves the endpoint implicitly via Steam lobbies.
    Steam,
}
