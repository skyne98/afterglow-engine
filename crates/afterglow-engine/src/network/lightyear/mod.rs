//! Lightyear integration boundary.

pub mod protocol;

pub use protocol::register_afterglow_lightyear_protocol;

use bevy::prelude::*;
#[cfg(feature = "lightyear")]
use lightyear::prelude::{
    Client, ConfirmedTick, Predicted, ReplicationReceiver, ReplicationSystems, RollbackSystems,
};
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
            let cfg = app.world().resource::<AfterglowLightyearConfig>().clone();
            let tick_duration = Duration::from_secs_f64(1.0 / cfg.tick_rate.max(1) as f64);

            match cfg.role {
                LightyearRole::Client => {
                    app.add_plugins(lightyear::prelude::client::ClientPlugins { tick_duration });
                    app.add_systems(
                        PreUpdate,
                        request_rollback_check_on_confirmed_tick_changed
                            .after(ReplicationSystems::Receive)
                            .before(RollbackSystems::Check),
                    );
                }
                LightyearRole::Server => {
                    app.add_plugins(lightyear::prelude::server::ServerPlugins { tick_duration });
                }
            }

            if !app.is_plugin_added::<InputManagerPlugin<AfterglowAction>>() {
                // This plugin registers Lightyear's native InputChannel and
                // InputMessage protocol. Do not register ActionState as a
                // normal replicated component; the input plugin owns its
                // tick-buffered transport and replay semantics.
                let mut input_plugin =
                    lightyear_inputs_leafwing::prelude::InputPlugin::<AfterglowAction>::default();
                input_plugin.config.rebroadcast_inputs = cfg.rebroadcast_inputs;
                app.add_plugins(input_plugin);
            }

            // Enable frame interpolation for Transform so predicted movement is
            // visually smooth between fixed ticks at any frame rate. Entities
            // must also receive the `FrameInterpolate<Transform>` component.
            app.add_plugins(lightyear::frame_interpolation::FrameInterpolationPlugin::<
                Transform,
            >::default());

            // Register engine-level protocol (StableEntityId, Transform,
            // LinearVelocity) with prediction and interpolation settings.
            register_afterglow_lightyear_protocol(app);
        }
    }
}

/// Last confirmed tick value for which Afterglow explicitly requested a
/// Lightyear rollback check.
#[cfg(feature = "lightyear")]
#[derive(Component, Clone, Copy, Debug, Eq, PartialEq)]
pub struct RollbackCheckedConfirmedTick(pub lightyear::prelude::Tick);

/// Lightyear sets `ReplicationReceiver::received_this_frame` when a state
/// packet is received, but ordered/buffered replication can apply the packet to
/// `Confirmed<T>` on a later frame. Rollback correctness depends on checking
/// prediction history when the confirmed tick actually changes, not only when
/// bytes arrive. This generic engine-side guard keeps gameplay systems from
/// copying confirmed state into live predicted physics bodies.
#[cfg(feature = "lightyear")]
pub fn request_rollback_check_on_confirmed_tick_changed(
    mut commands: Commands,
    mut receivers: Query<&mut ReplicationReceiver, With<Client>>,
    mut confirmed_entities: Query<
        (
            Entity,
            &mut ConfirmedTick,
            Option<&RollbackCheckedConfirmedTick>,
        ),
        With<Predicted>,
    >,
) {
    let mut needs_rollback_check = false;
    for (entity, mut confirmed_tick, last_checked) in &mut confirmed_entities {
        if last_checked.is_some_and(|last| last.0 == confirmed_tick.tick) {
            continue;
        }
        // Lightyear's rollback checker additionally filters on
        // `ConfirmedTick` change detection. Mark the component changed when the
        // actual tick value advances, independent of whether Bevy observed the
        // original mutation in this schedule.
        confirmed_tick.set_changed();
        commands
            .entity(entity)
            .insert(RollbackCheckedConfirmedTick(confirmed_tick.tick));
        needs_rollback_check = true;
    }
    if needs_rollback_check {
        for mut receiver in &mut receivers {
            receiver.received_this_frame = true;
        }
    }
}
