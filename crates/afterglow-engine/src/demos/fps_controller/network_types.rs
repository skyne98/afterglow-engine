use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::identity::StableEntityId;

#[derive(Resource, Clone, Debug, Eq, PartialEq)]
pub struct FpsDemoNetworkConfig {
    pub launch: FpsDemoLaunchMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FpsDemoLaunchMode {
    Local,
    Remote(String),
    Server(String),
}

#[derive(Resource, Clone, Debug, Eq, PartialEq)]
pub struct FpsDemoNetworkStatus {
    pub connection: FpsDemoConnectionState,
    pub local_server_running: bool,
    pub lightyear_links: bool,
    pub replicated_avatar: bool,
    pub replicated_avatar_count: usize,
    pub visible_remote_avatar_count: usize,
    pub local_player_round_trip: bool,
    pub latency_ms: u32,
    pub ticks: u32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum FpsDemoConnectionState {
    #[default]
    Disconnected,
    Local,
    Remote(String),
    Server(String),
}

#[derive(Component, Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct FpsDemoPlayerState {
    pub position_mm: [i32; 3],
    pub yaw_milliradians: i32,
    pub pitch_milliradians: i32,
    pub authoritative_tick: u32,
}

#[derive(Component, Clone, Copy, Debug, Eq, PartialEq)]
pub struct FpsDemoRemoteAvatar {
    pub stable_id: StableEntityId,
}

impl Default for FpsDemoNetworkConfig {
    fn default() -> Self {
        Self {
            launch: FpsDemoLaunchMode::Local,
        }
    }
}

impl Default for FpsDemoNetworkStatus {
    fn default() -> Self {
        Self {
            connection: FpsDemoConnectionState::Disconnected,
            local_server_running: false,
            lightyear_links: false,
            replicated_avatar: false,
            replicated_avatar_count: 0,
            visible_remote_avatar_count: 0,
            local_player_round_trip: false,
            latency_ms: 0,
            ticks: 0,
        }
    }
}

impl FpsDemoNetworkConfig {
    pub fn local() -> Self {
        Self {
            launch: FpsDemoLaunchMode::Local,
        }
    }

    pub fn remote(addr: impl Into<String>) -> Self {
        Self {
            launch: FpsDemoLaunchMode::Remote(addr.into()),
        }
    }

    pub fn server(addr: impl Into<String>) -> Self {
        Self {
            launch: FpsDemoLaunchMode::Server(addr.into()),
        }
    }
}

impl FpsDemoPlayerState {
    pub fn from_translation(translation: Vec3) -> Self {
        Self {
            position_mm: [
                meters_to_millimeters(translation.x),
                meters_to_millimeters(translation.y),
                meters_to_millimeters(translation.z),
            ],
            yaw_milliradians: 0,
            pitch_milliradians: 0,
            authoritative_tick: 0,
        }
    }

    #[cfg_attr(not(feature = "lightyear"), allow(dead_code))]
    pub fn to_translation(&self) -> Vec3 {
        Vec3::new(
            self.position_mm[0] as f32 / 1000.0,
            self.position_mm[1] as f32 / 1000.0,
            self.position_mm[2] as f32 / 1000.0,
        )
    }
}

fn meters_to_millimeters(value: f32) -> i32 {
    (value * 1000.0).round() as i32
}
