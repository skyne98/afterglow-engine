//! Lightyear integration boundary.

#[cfg(feature = "lightyear")]
pub mod link;

pub mod protocol;

#[cfg(feature = "lightyear")]
pub use link::{
    AfterglowNetcodeConsumerPlugin, AfterglowSessionLightyearBridgePlugin, NetcodeClientParams,
    NetcodeServerParams, PendingNetcodeStartup, SessionLightyearLinks,
};

pub use protocol::register_afterglow_lightyear_protocol;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
#[cfg(feature = "lightyear")]
use std::time::Duration;

#[cfg(feature = "lightyear")]
use crate::input::AfterglowAction;
#[cfg(feature = "lightyear")]
use leafwing_input_manager::plugin::InputManagerPlugin;

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
    pub netcode_private_key: [u8; 32],
    /// Fixed input delay (in ticks) applied to client links for server-side
    /// consumption. Keeps local predicted presentation immediate while
    /// delaying server input processing.
    pub input_delay_ticks: u16,
    /// If true, client inputs are rebroadcast to other clients so they can
    /// predict remote players' actions.
    pub rebroadcast_inputs: bool,
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
            netcode_private_key: [0u8; 32],
            input_delay_ticks: 2,
            rebroadcast_inputs: true,
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
            use lightyear::prelude::*;

            let cfg = app.world().resource::<AfterglowLightyearConfig>().clone();
            let tick_duration = Duration::from_secs_f64(1.0 / cfg.tick_rate.max(1) as f64);

            app.add_channel::<lightyear::input::InputChannel>(ChannelSettings {
                mode: ChannelMode::UnorderedUnreliable,
                send_frequency: Duration::default(),
                priority: f32::INFINITY,
            })
            .add_direction(NetworkDirection::Bidirectional);

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

            if !app.is_plugin_added::<InputManagerPlugin<AfterglowAction>>() {
                app.add_plugins(lightyear_inputs_leafwing::prelude::InputPlugin::<
                    AfterglowAction,
                >::default());
            }

            // Enable rebroadcast and input delay from config so every game
            // gets sensible defaults without demo-local boilerplate.
            app.add_systems(Update, configure_input_defaults);

            // Enable frame interpolation for Transform so predicted movement is
            // visually smooth between fixed ticks at any frame rate. Entities
            // must also receive the `FrameInterpolate<Transform>` component.
            app.add_plugins(lightyear::frame_interpolation::FrameInterpolationPlugin::<
                Transform,
            >::default());

            // Lightyear doesn't auto-add a ReplicationSender to entities that
            // gain a LinkOf component (see `handle_new_client` in the
            // `lightyear` crate's lib.rs doctest). Server-side replication
            // would then silently skip them. Register the observer here so
            // every incoming remote-client connection gets a sender.
            app.add_observer(add_replication_sender_on_link_of);
            app.add_systems(PreUpdate, ensure_replication_channels);
        }
    }
}

/// Marker component to track that we've configured the input delay on a
/// client link. `InputTimelineConfig` is a required component of `Client`,
/// so checking for its existence doesn't work — it's always present from
/// spawn. We need this marker to avoid re-inserting `InputTimelineConfig`
/// every frame (which would trigger the config-update observer repeatedly).
#[derive(Component)]
struct InputDelayConfigured;

/// Applies `rebroadcast_inputs` and `input_delay_ticks` from
/// [`AfterglowLightyearConfig`] to Lightyear's input config resources and
/// client link entities.
#[cfg(feature = "lightyear")]
fn configure_input_defaults(
    config: Res<AfterglowLightyearConfig>,
    mut client_config: Option<ResMut<lightyear::prelude::input::InputConfig<AfterglowAction>>>,
    mut server_config: Option<
        ResMut<lightyear::prelude::input::server::ServerInputConfig<AfterglowAction>>,
    >,
    links: Option<Res<SessionLightyearLinks>>,
    configured: Query<(), With<InputDelayConfigured>>,
    mut commands: Commands,
) {
    if let Some(ref mut client_config) = client_config {
        client_config.rebroadcast_inputs = config.rebroadcast_inputs;
    }
    if let Some(ref mut server_config) = server_config {
        server_config.rebroadcast_inputs = config.rebroadcast_inputs;
    }

    let Some(client_link) = links.and_then(|links| links.client_link) else {
        return;
    };
    if configured.get(client_link).is_ok() {
        return;
    }
    // Replace the default InputTimelineConfig (which has no input delay) with
    // one that has the configured delay. This is critical: without input delay,
    // lost input packets create gaps in the server's InputBuffer, causing stale
    // ActionState, jitter, and "stuck" inputs.
    commands.entity(client_link).insert((
        lightyear::prelude::client::InputTimelineConfig::default().with_input_delay(
            lightyear::prelude::client::InputDelayConfig::fixed_input_delay(
                config.input_delay_ticks,
            ),
        ),
        InputDelayConfigured,
    ));
}

/// Adds a [`ReplicationSender`] to any entity that gains a [`LinkOf`]
/// component, so the server-side replication stream can route to it.
///
/// Does NOT insert a `Transport` — if the entity already has one (e.g. from
/// `NetcodeServerPlugin`), inserting a bare `Transport::default()` would
/// overwrite it and break the netcode connection. Instead,
/// [`ensure_replication_channels`] runs as a system to add the missing
/// channel senders/receivers to the existing `Transport`.
///
/// Skips entities that already have a `ReplicationSender` so the host's
/// own loopback client (which the consumer plugin sets up directly) isn't
/// double-configured.
#[cfg(feature = "lightyear")]
fn add_replication_sender_on_link_of(
    trigger: bevy::prelude::On<bevy::prelude::Add, lightyear::prelude::LinkOf>,
    mut commands: bevy::prelude::Commands,
    existing_senders: bevy::prelude::Query<
        (),
        bevy::prelude::With<lightyear::prelude::ReplicationSender>,
    >,
) {
    use bevy::prelude::*;
    use lightyear::prelude::*;
    use std::time::Duration;

    if existing_senders.get(trigger.entity).is_ok() {
        return;
    }
    commands.entity(trigger.entity).insert((
        ReplicationSender::new(
            Duration::from_millis(16),
            SendUpdatesMode::SinceLastAck,
            false,
        ),
        Name::from("RemoteClient"),
    ));
}

/// Ensures every `LinkOf` entity's `Transport` has senders/receivers for the
/// replication and input channels. Runs every frame.
///
/// Only adds channels that are missing — `add_sender_from_registry` uses
/// `HashMap::insert` which would overwrite existing channel state (acks,
/// nacks, messages_sent) if called again, causing "Received an update
/// message-id ack but we don't know the corresponding group id" errors.
#[cfg(feature = "lightyear")]
fn ensure_replication_channels(
    registry: Res<lightyear::prelude::ChannelRegistry>,
    links: Query<
        Entity,
        (
            With<lightyear::prelude::LinkOf>,
            With<lightyear::prelude::ReplicationSender>,
        ),
    >,
    mut transports: Query<&mut lightyear::prelude::Transport>,
) {
    use lightyear::prelude::*;
    for entity in &links {
        let Ok(mut transport) = transports.get_mut(entity) else {
            continue;
        };
        if !transport.has_sender::<MetadataChannel>() {
            transport.add_sender_from_registry::<MetadataChannel>(&registry);
        }
        if !transport.has_receiver::<MetadataChannel>() {
            transport.add_receiver_from_registry::<MetadataChannel>(&registry);
        }
        if !transport.has_sender::<UpdatesChannel>() {
            transport.add_sender_from_registry::<UpdatesChannel>(&registry);
        }
        if !transport.has_receiver::<UpdatesChannel>() {
            transport.add_receiver_from_registry::<UpdatesChannel>(&registry);
        }
        if !transport.has_sender::<ActionsChannel>() {
            transport.add_sender_from_registry::<ActionsChannel>(&registry);
        }
        if !transport.has_receiver::<ActionsChannel>() {
            transport.add_receiver_from_registry::<ActionsChannel>(&registry);
        }
        if registry
            .settings(lightyear_transport::channel::ChannelKind::of::<
                lightyear::input::InputChannel,
            >())
            .is_some()
        {
            if !transport.has_sender::<lightyear::input::InputChannel>() {
                transport.add_sender_from_registry::<lightyear::input::InputChannel>(&registry);
            }
            if !transport.has_receiver::<lightyear::input::InputChannel>() {
                transport.add_receiver_from_registry::<lightyear::input::InputChannel>(&registry);
            }
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
        assert_eq!(cfg.input_delay_ticks, 2);
        assert!(cfg.rebroadcast_inputs);
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

    #[cfg(feature = "lightyear")]
    #[test]
    fn host_role_adds_lightyear_plugins_without_panicking() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(AfterglowLightyearConfig {
                role: LightyearRole::Host,
                ..Default::default()
            });

        app.add_plugins(AfterglowLightyearPlugin);
    }

    #[cfg(feature = "lightyear")]
    #[test]
    fn frame_interpolation_plugin_is_registered() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(AfterglowLightyearConfig::default());
        app.add_plugins(AfterglowLightyearPlugin);
        // FrameInterpolationPlugin<Transform> should be registered by the engine
        assert!(
            app.is_plugin_added::<lightyear::frame_interpolation::FrameInterpolationPlugin<
                bevy::transform::components::Transform,
            >>()
        );
    }
}
