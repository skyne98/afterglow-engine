//! Lightyear integration boundary.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
#[cfg(feature = "lightyear")]
use std::time::Duration;

#[cfg(feature = "lightyear")]
use crate::input::AfterglowAction;

/// Runtime role used by [`AfterglowLightyearConfig`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum LightyearRole {
    #[default]
    Client,
    Server,
    Host,
}

/// Top-level config for the Lightyear networking layer.
#[derive(Resource, Clone, Debug)]
pub struct AfterglowLightyearConfig {
    pub role: LightyearRole,
    pub server_addr: String,
    pub remote_addr: String,
    pub tick_rate: u64,
    pub predicted_ticks: u32,
    pub link_conditioner: Option<LightyearLinkConditioner>,
    pub connect_token: Option<Vec<u8>>,
    pub protocol_id: u64,
}

impl Default for AfterglowLightyearConfig {
    fn default() -> Self {
        Self {
            role: LightyearRole::Client,
            server_addr: "0.0.0.0:0".into(),
            remote_addr: "127.0.0.1:8820".into(),
            tick_rate: 60,
            predicted_ticks: 12,
            link_conditioner: None,
            connect_token: None,
            protocol_id: 0,
        }
    }
}

/// Serializable link conditioner policy. Transport-specific wiring is added
/// when the first real link entities are spawned.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LightyearLinkConditioner {
    pub incoming_latency_ms: u32,
    pub incoming_jitter_ms: u32,
    pub incoming_loss: f32,
    pub outgoing_latency_ms: u32,
    pub outgoing_jitter_ms: u32,
    pub outgoing_loss: f32,
}

/// Bevy plugin that registers the Lightyear plugin groups selected by
/// [`AfterglowLightyearConfig`]. Link entities and concrete transports are
/// intentionally configured in later migration slices.
pub struct AfterglowLightyearPlugin;

impl Plugin for AfterglowLightyearPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AfterglowLightyearConfig>();

        #[cfg(feature = "lightyear")]
        {
            let cfg = app.world().resource::<AfterglowLightyearConfig>().clone();
            let tick_duration = Duration::from_secs_f64(1.0 / cfg.tick_rate.max(1) as f64);

            match cfg.role {
                LightyearRole::Client => {
                    app.add_plugins(lightyear::prelude::client::ClientPlugins { tick_duration });
                }
                LightyearRole::Server => {
                    app.add_plugins(lightyear::prelude::server::ServerPlugins { tick_duration });
                }
                LightyearRole::Host => {
                    app.add_plugins((
                        lightyear::prelude::server::ServerPlugins { tick_duration },
                        lightyear::prelude::client::ClientPlugins { tick_duration },
                    ));
                }
            }

            app.add_plugins(lightyear_inputs_leafwing::prelude::InputPlugin::<
                AfterglowAction,
            >::default());
        }
    }
}

// --------------------------------------------------------------------------
// Unit tests (cfg-gated)
// --------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_are_sensible() {
        let cfg = AfterglowLightyearConfig::default();
        assert_eq!(cfg.role, LightyearRole::Client);
        assert!(cfg.tick_rate > 0);
        assert!(cfg.predicted_ticks > 0);
    }

    #[test]
    fn config_is_a_resource() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AfterglowLightyearPlugin);
        assert!(app.world().contains_resource::<AfterglowLightyearConfig>());
    }

    #[test]
    fn tick_duration_does_not_divide_by_zero() {
        let mut cfg = AfterglowLightyearConfig::default();
        cfg.tick_rate = 0;
        assert_eq!(cfg.tick_rate.max(1), 1);
    }
}
