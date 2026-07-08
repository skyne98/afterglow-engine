use super::{
    ConnectionEvent, ConnectionEventKind, MemberLinkMap, PlayerId,
    auth::{ChallengeMessage, PendingAuth, on_client_disconnected, on_client_of_added},
    readiness::register_clientof_required_components,
};
use bevy::prelude::*;
use lightyear::prelude::*;
use std::time::Duration;

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
fn on_client_of_added_configures_required_sender_transport_and_auth() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(lightyear::prelude::server::ServerPlugins {
        tick_duration: Duration::from_secs_f64(1.0 / 60.0),
    });

    register_clientof_required_components(&mut app);
    app.init_resource::<MemberLinkMap>();
    app.register_message::<ChallengeMessage>()
        .add_direction(NetworkDirection::ServerToClient);
    app.add_observer(on_client_of_added);

    let entity = app.world_mut().spawn((RemoteId(PeerId::Netcode(1)),)).id();
    app.world_mut()
        .commands()
        .entity(entity)
        .insert(lightyear::prelude::server::ClientOf);
    app.update();

    let world = app.world();
    let transport = world
        .entity(entity)
        .get::<Transport>()
        .expect("Transport should be present on the ClientOf link entity");
    assert!(world.entity(entity).contains::<ReplicationSender>());
    assert!(world.entity(entity).contains::<PendingAuth>());
    assert!(!world.entity(entity).contains::<Disconnected>());
    assert_standard_channels(transport);
}

#[derive(Resource, Default)]
struct SeenDisconnects(Vec<(PlayerId, String)>);

fn record_disconnect(trigger: On<ConnectionEvent>, mut seen: ResMut<SeenDisconnects>) {
    if let ConnectionEventKind::Disconnected { reason } = &trigger.event().kind {
        seen.0.push((trigger.event().player_id, reason.clone()));
    }
}

#[cfg(feature = "lightyear")]
#[test]
fn client_disconnected_emits_connection_event_and_clears_member_link() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<lightyear::prelude::PeerMetadata>();
    app.init_resource::<MemberLinkMap>();
    app.init_resource::<SeenDisconnects>();
    app.add_observer(on_client_disconnected);
    app.add_observer(record_disconnect);

    let entity = app
        .world_mut()
        .spawn((
            RemoteId(PeerId::Netcode(7)),
            lightyear::prelude::server::ClientOf,
        ))
        .id();
    app.world_mut()
        .resource_mut::<MemberLinkMap>()
        .links
        .insert(7, entity);

    app.world_mut()
        .commands()
        .entity(entity)
        .insert(Disconnected {
            reason: Some("test disconnect".into()),
        });
    app.update();

    assert!(
        !app.world()
            .resource::<MemberLinkMap>()
            .links
            .contains_key(&7)
    );
    assert_eq!(
        app.world().resource::<SeenDisconnects>().0,
        vec![(7, "test disconnect".to_string())]
    );
}
