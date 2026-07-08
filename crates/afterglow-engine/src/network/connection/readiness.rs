//! Server-side readiness repairs for Lightyear `ClientOf` link entities.
//!
//! Lightyear may add a new `ClientOf` to `Server::collection()` before any
//! game observer commands have flushed. Replicated entities using
//! `Replicate::to_clients(NetworkTarget::All)` inspect that collection in
//! component hooks, so the `ReplicationSender` must exist as part of the same
//! structural insertion as `ClientOf`, not one deferred command later.

use std::time::Duration;

use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear_transport::channel::ChannelKind;

fn default_replication_sender() -> ReplicationSender {
    ReplicationSender::new(
        Duration::from_millis(16),
        SendUpdatesMode::SinceLastAck,
        false,
    )
}

pub(crate) fn register_clientof_required_components(app: &mut App) {
    let _ = app
        .world_mut()
        .try_register_required_components_with::<
            lightyear::prelude::server::ClientOf,
            ReplicationSender,
        >(default_replication_sender);
}

/// Idempotent server-side safety net for `ClientOf` links.
///
/// Required components make `ReplicationSender` available immediately when
/// `ClientOf` is inserted. This system additionally configures transport
/// channels and repairs manually spawned test links that bypassed the plugin.
pub(crate) fn ensure_client_replication_senders_ready(
    mut commands: Commands,
    registry: Option<Res<ChannelRegistry>>,
    mut links: Query<
        (
            Entity,
            Option<&RemoteId>,
            Option<&ReplicationSender>,
            Option<&mut Transport>,
        ),
        With<lightyear::prelude::server::ClientOf>,
    >,
) {
    let Some(registry) = registry else {
        return;
    };

    for (entity, remote_id, sender, transport) in &mut links {
        let player_id = match remote_id.map(|id| &id.0) {
            Some(PeerId::Netcode(id)) => *id,
            _ => 0,
        };
        let name = Name::from(format!("RemoteClient-{player_id}"));

        match (sender.is_some(), transport) {
            (true, Some(mut transport)) => {
                add_channels_to_transport(&mut transport, &registry);
                commands.entity(entity).insert(name);
            }
            (true, None) => {
                let mut transport = Transport::default();
                add_channels_to_transport(&mut transport, &registry);
                commands.entity(entity).insert((name, transport));
            }
            (false, Some(mut transport)) => {
                add_channels_to_transport(&mut transport, &registry);
                commands
                    .entity(entity)
                    .insert((default_replication_sender(), name));
            }
            (false, None) => {
                let mut transport = Transport::default();
                add_channels_to_transport(&mut transport, &registry);
                commands
                    .entity(entity)
                    .insert((default_replication_sender(), name, transport));
            }
        }
    }
}

/// Adds the standard replication/input channels to a [`Transport`].
///
/// Idempotent: skips channels already present.
pub(crate) fn add_channels_to_transport(transport: &mut Transport, registry: &ChannelRegistry) {
    if !transport.has_sender::<MetadataChannel>() {
        transport.add_sender_from_registry::<MetadataChannel>(registry);
    }
    if !transport.has_receiver::<MetadataChannel>() {
        transport.add_receiver_from_registry::<MetadataChannel>(registry);
    }
    if !transport.has_sender::<UpdatesChannel>() {
        transport.add_sender_from_registry::<UpdatesChannel>(registry);
    }
    if !transport.has_receiver::<UpdatesChannel>() {
        transport.add_receiver_from_registry::<UpdatesChannel>(registry);
    }
    if !transport.has_sender::<ActionsChannel>() {
        transport.add_sender_from_registry::<ActionsChannel>(registry);
    }
    if !transport.has_receiver::<ActionsChannel>() {
        transport.add_receiver_from_registry::<ActionsChannel>(registry);
    }
    if registry
        .settings(ChannelKind::of::<lightyear::input::InputChannel>())
        .is_some()
    {
        if !transport.has_sender::<lightyear::input::InputChannel>() {
            transport.add_sender_from_registry::<lightyear::input::InputChannel>(registry);
        }
        if !transport.has_receiver::<lightyear::input::InputChannel>() {
            transport.add_receiver_from_registry::<lightyear::input::InputChannel>(registry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_standard_channels(transport: &Transport) {
        assert!(transport.has_sender::<MetadataChannel>());
        assert!(transport.has_receiver::<MetadataChannel>());
        assert!(transport.has_sender::<UpdatesChannel>());
        assert!(transport.has_receiver::<UpdatesChannel>());
        assert!(transport.has_sender::<ActionsChannel>());
        assert!(transport.has_receiver::<ActionsChannel>());
    }

    #[cfg(feature = "lightyear")]
    #[test]
    fn clientof_required_components_insert_replication_sender_immediately() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        register_clientof_required_components(&mut app);

        let entity = app
            .world_mut()
            .spawn(lightyear::prelude::server::ClientOf)
            .id();

        assert!(app.world().entity(entity).contains::<ReplicationSender>());
        assert!(app.world().entity(entity).contains::<Transport>());
    }

    #[cfg(feature = "lightyear")]
    #[test]
    fn readiness_system_wires_clientof_before_replication_buffer() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(lightyear::prelude::server::ServerPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / 60.0),
        });
        app.add_systems(Update, ensure_client_replication_senders_ready);
        app.finish();
        app.cleanup();

        let entity = app
            .world_mut()
            .spawn((
                RemoteId(PeerId::Netcode(183)),
                lightyear::prelude::server::ClientOf,
            ))
            .id();

        app.update();

        let world = app.world();
        assert!(world.entity(entity).contains::<ReplicationSender>());
        let transport = world
            .entity(entity)
            .get::<Transport>()
            .expect("missing transport should be inserted and pre-wired");
        assert_standard_channels(transport);
    }
}
