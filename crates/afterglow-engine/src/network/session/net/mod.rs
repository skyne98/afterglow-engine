//! Networked session provider / client for the NonSteam backend.
//!
//! The provider listens for `SessionRequest` control messages over TCP and
//! replies with `SessionEvent`s. The client sends requests to a remote
//! provider and surfaces the responses as local `SessionEvent` messages.

pub mod client;
pub mod protocol;
pub mod provider;

pub use client::{NonSteamSessionClient};
pub use provider::{NonSteamSessionProvider};
