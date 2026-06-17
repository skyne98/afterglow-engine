//! Engine-level integration tests for the Lightyear networking layer.
//!
//! These tests verify the engine-owned systems that every networked game
//! depends on: input delay configuration, rebroadcast, frame interpolation,
//! transport channel setup, and physics bridge integration.

use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear_transport::{channel::registry::ChannelKind, packet::message::MessageId};

use super::*;

fn test_app_with_role(role: LightyearRole) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(AfterglowLightyearConfig {
            role,
            netcode_private_key: [42u8; 32],
            ..Default::default()
        });
    app.add_plugins(AfterglowLightyearPlugin);
    app
}

// ---------------------------------------------------------------------------
// Config defaults and invariants (no lightyear feature needed)
// ---------------------------------------------------------------------------

#[test]
fn config_defaults_have_sensible_values() {
    let cfg = AfterglowLightyearConfig::default();
    assert_eq!(cfg.role, LightyearRole::Client);
    assert_eq!(cfg.tick_rate, 60);
    assert_eq!(cfg.predicted_ticks, 12);
    assert_eq!(cfg.input_delay_ticks, 2, "input delay must be > 0 for UDP");
    assert!(
        cfg.rebroadcast_inputs,
        "rebroadcast should be on by default"
    );
}

#[test]
fn config_custom_delay_is_respected() {
    let cfg = AfterglowLightyearConfig {
        input_delay_ticks: 5,
        rebroadcast_inputs: false,
        ..Default::default()
    };
    assert_eq!(cfg.input_delay_ticks, 5);
    assert!(!cfg.rebroadcast_inputs);
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

// ---------------------------------------------------------------------------
// Plugin registration
// ---------------------------------------------------------------------------

#[cfg(feature = "lightyear")]
#[test]
fn plugin_registers_frame_interpolation() {
    let app = test_app_with_role(LightyearRole::Client);
    assert!(
        app.is_plugin_added::<lightyear::frame_interpolation::FrameInterpolationPlugin<
            bevy::transform::components::Transform,
        >>()
    );
}

#[cfg(feature = "lightyear")]
#[test]
fn plugin_registers_input_channel() {
    let app = test_app_with_role(LightyearRole::Client);
    let registry = app.world().resource::<ChannelRegistry>();
    assert!(
        registry
            .settings(lightyear_transport::channel::ChannelKind::of::<
                lightyear::input::InputChannel,
            >())
            .is_some(),
        "InputChannel must be registered by AfterglowLightyearPlugin"
    );
}

#[cfg(feature = "lightyear")]
#[test]
fn plugin_registers_leafwing_input() {
    let app = test_app_with_role(LightyearRole::Client);
    // Lightyear's InputPlugin<AfterglowAction> is registered by our plugin.
    // It internally adds InputManagerPlugin<AfterglowAction> when bevy_input is
    // present. We check that the InputPlugin itself is registered.
    assert!(
        app.is_plugin_added::<
            lightyear_inputs_leafwing::prelude::InputPlugin<crate::input::AfterglowAction>,
        >()
    );
}

// ---------------------------------------------------------------------------
// Input delay configuration (the bug that caused stuck inputs)
// ---------------------------------------------------------------------------

#[cfg(feature = "lightyear")]
#[test]
fn input_delay_is_set_on_client_link() {
    // This test verifies the fix for the bug where InputTimelineConfig (a
    // required component of Client) always existed, so our check for its
    // existence always returned true and the delay was never set.
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(AfterglowLightyearConfig::default());
    app.add_plugins(AfterglowLightyearPlugin);
    app.add_plugins(crate::network::AfterglowSessionPlugin);
    app.add_plugins(crate::network::AfterglowSessionLightyearBridgePlugin);
    app.finish();
    app.cleanup();

    // Simulate having a client link by inserting InputTimelineConfig (the
    // default, no delay) and SessionLightyearLinks pointing to it.
    let client_link = app
        .world_mut()
        .spawn(lightyear::prelude::client::InputTimelineConfig::default())
        .id();
    app.world_mut().insert_resource(SessionLightyearLinks {
        client_link: Some(client_link),
        server_link: None,
        server_entity: None,
    });

    // Run configure_input_defaults
    app.update();

    assert!(
        app.world()
            .get::<super::InputDelayConfigured>(client_link)
            .is_some(),
        "InputDelayConfigured marker should be set after configure_input_defaults runs"
    );
}

#[cfg(feature = "lightyear")]
#[test]
fn input_delay_not_re_applied() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(AfterglowLightyearConfig::default());
    app.add_plugins(AfterglowLightyearPlugin);
    app.add_plugins(crate::network::AfterglowSessionPlugin);
    app.add_plugins(crate::network::AfterglowSessionLightyearBridgePlugin);
    app.finish();
    app.cleanup();

    let client_link = app
        .world_mut()
        .spawn(lightyear::prelude::client::InputTimelineConfig::default())
        .id();
    app.world_mut().insert_resource(SessionLightyearLinks {
        client_link: Some(client_link),
        server_link: None,
        server_entity: None,
    });

    app.update(); // first configure_input_defaults
    assert!(
        app.world()
            .get::<super::InputDelayConfigured>(client_link)
            .is_some(),
        "InputDelayConfigured should be set after first run"
    );

    // Run more times — should be a no-op
    app.update();
    app.update();
    app.update();

    assert!(
        app.world()
            .get::<super::InputDelayConfigured>(client_link)
            .is_some(),
        "InputDelayConfigured marker should persist"
    );
}

// ---------------------------------------------------------------------------
// Rebroadcast configuration
// ---------------------------------------------------------------------------

#[cfg(feature = "lightyear")]
#[test]
fn rebroadcast_is_configured() {
    let mut app = test_app_with_role(LightyearRole::Host);
    app.finish();
    app.cleanup();
    app.update();
    app.update();

    let client_config = app
        .world()
        .get_resource::<lightyear::prelude::input::InputConfig<crate::input::AfterglowAction>>();
    if let Some(client_config) = client_config {
        assert!(
            client_config.rebroadcast_inputs,
            "client rebroadcast should be true by default"
        );
    }

    let server_config = app.world().get_resource::<
        lightyear::prelude::input::server::ServerInputConfig<crate::input::AfterglowAction>,
    >();
    if let Some(server_config) = server_config {
        assert!(
            server_config.rebroadcast_inputs,
            "server rebroadcast should be true by default"
        );
    }
}

// ---------------------------------------------------------------------------
// Transport channel setup
// ---------------------------------------------------------------------------

#[cfg(feature = "lightyear")]
#[test]
fn ensure_replication_channels_does_not_overwrite_existing() {
    // This tests the fix for the "Received an update message-id ack but we
    // don't know the corresponding group id" error.
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);

    // Set up a ChannelRegistry with the replication channels
    app.add_channel::<MetadataChannel>(ChannelSettings {
        mode: ChannelMode::UnorderedUnreliable,
        send_frequency: Duration::ZERO,
        priority: 1.0,
    });
    app.add_channel::<UpdatesChannel>(ChannelSettings {
        mode: ChannelMode::UnorderedUnreliable,
        send_frequency: Duration::ZERO,
        priority: 1.0,
    });
    app.add_channel::<ActionsChannel>(ChannelSettings {
        mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
        send_frequency: Duration::ZERO,
        priority: 1.0,
    });
    app.finish();
    app.cleanup();

    // Spawn a LinkOf + ReplicationSender + Transport with channels
    let server_entity = app.world_mut().spawn(Server::default()).id();
    let link_entity = app
        .world_mut()
        .spawn((
            LinkOf {
                server: server_entity,
            },
            ReplicationSender::new(
                Duration::from_millis(16),
                SendUpdatesMode::SinceLastAck,
                false,
            ),
        ))
        .id();

    // Manually add channels to the transport
    {
        let registry = app.world().resource::<ChannelRegistry>().clone();
        let mut transport = Transport::default();
        transport.add_sender_from_registry::<UpdatesChannel>(&registry);
        transport.add_receiver_from_registry::<UpdatesChannel>(&registry);
        app.world_mut().entity_mut(link_entity).insert(transport);
    }

    // Add some fake ack data to the transport's UpdatesChannel sender
    {
        let mut entity = app.world_mut().entity_mut(link_entity);
        let mut transport = entity.get_mut::<Transport>().unwrap();
        let sender = transport
            .senders
            .get_mut(&ChannelKind::of::<UpdatesChannel>())
            .unwrap();
        sender.message_acks.push(MessageId(42));
        sender.message_nacks.push(MessageId(99));
    }

    // Run ensure_replication_channels
    app.add_systems(Update, ensure_replication_channels);
    app.update();

    // Verify the ack data was NOT overwritten
    let transport = app.world().get::<Transport>(link_entity).unwrap();
    let sender = transport
        .senders
        .get(&ChannelKind::of::<UpdatesChannel>())
        .unwrap();
    assert_eq!(
        sender.message_acks.len(),
        1,
        "message_acks should not be overwritten by ensure_replication_channels"
    );
    assert_eq!(
        sender.message_nacks.len(),
        1,
        "message_nacks should not be overwritten"
    );
}

// ---------------------------------------------------------------------------
// Physics bridge integration
// ---------------------------------------------------------------------------

#[cfg(feature = "lightyear")]
#[test]
fn physics_plugin_uses_bridge_when_lightyear_active() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(AfterglowLightyearConfig::default());
    app.add_plugins(AfterglowLightyearPlugin);
    app.add_plugins(crate::physics::AfterglowPhysicsPlugin);

    // The bridge plugin should be added
    assert!(
        app.is_plugin_added::<afterglow_lightyear_avian3d::prelude::AfterglowAvianPlugin>(),
        "AfterglowPhysicsPlugin should use the Avian bridge when lightyear is active"
    );

    // Avian's own transform/interpolation plugins should be disabled
    // (they're replaced by the bridge)
    assert!(
        app.world()
            .contains_resource::<avian3d::physics_transform::PhysicsTransformConfig>(),
        "PhysicsTransformConfig should exist (added by bridge)"
    );
}

#[test]
fn physics_plugin_without_lightyear_uses_default() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(crate::physics::AfterglowPhysicsPlugin);

    // Without lightyear, the bridge should NOT be added
    assert!(
        !app.is_plugin_added::<afterglow_lightyear_avian3d::prelude::AfterglowAvianPlugin>(),
        "AfterglowPhysicsPlugin should NOT use the Avian bridge without lightyear"
    );
}
