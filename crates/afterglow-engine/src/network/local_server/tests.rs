use super::*;
use crate::{
    input::{AfterglowInputPlugin, InputActionValue, VirtualInputState},
    network::{AfterglowNetworkPlugin, NetworkPlayerId, authority::CommandRejectReason},
    testing::unit_app,
};

#[test]
fn disabled_local_server_does_not_create_session_or_submit_commands() {
    let mut app = unit_app();
    app.add_plugins((AfterglowInputPlugin, AfterglowNetworkPlugin));
    app.world_mut()
        .resource_mut::<VirtualInputState>()
        .press_action("use");

    app.update();

    let session = app.world().resource::<NetworkSession>();
    assert!(session.peer(PeerId(0)).is_none());
    assert!(
        app.world()
            .resource::<ServerCommandBuffer>()
            .accepted()
            .is_empty()
    );
}

#[test]
fn local_server_without_input_resources_is_a_noop() {
    let mut app = unit_app();
    app.add_plugins(AfterglowNetworkPlugin)
        .insert_resource(LocalServerConfig::single_player());

    app.update();

    let session = app.world().resource::<NetworkSession>();
    assert!(session.peer(PeerId(0)).is_none());
    assert!(
        app.world()
            .resource::<ServerCommandBuffer>()
            .accepted()
            .is_empty()
    );
}

#[test]
fn disabling_local_server_clears_local_session_state() {
    let mut app = unit_app();
    app.add_plugins((AfterglowInputPlugin, AfterglowNetworkPlugin))
        .insert_resource(LocalServerConfig::single_player());
    app.update();
    app.world_mut().resource_mut::<LocalServerConfig>().enabled = false;

    app.update();

    assert_eq!(app.world().resource::<LocalPlayers>().peer, None);
    assert!(
        app.world()
            .resource::<NetworkSession>()
            .peer(PeerId(0))
            .is_none()
    );
}

#[test]
fn changing_local_server_peer_clears_previous_owned_peer() {
    let mut app = unit_app();
    app.add_plugins((AfterglowInputPlugin, AfterglowNetworkPlugin))
        .insert_resource(LocalServerConfig::single_player());
    app.update();
    app.world_mut().resource_mut::<LocalServerConfig>().peer = PeerId(4);

    app.update();

    let session = app.world().resource::<NetworkSession>();
    assert!(session.peer(PeerId(0)).is_none());
    assert!(session.owns_player(PeerId(4), NetworkPlayerId(1)));
    assert_eq!(app.world().resource::<LocalPlayers>().peer, Some(PeerId(4)));
}

#[test]
fn single_player_multiplayer_single_player_transition_is_clean() {
    let mut app = unit_app();
    app.add_plugins((AfterglowInputPlugin, AfterglowNetworkPlugin))
        .insert_resource(LocalServerConfig::single_player());
    app.update();
    app.world_mut().resource_mut::<LocalServerConfig>().enabled = false;
    app.update();
    {
        let mut session = app.world_mut().resource_mut::<NetworkSession>();
        session.connect_peer(
            PeerId(20),
            PlatformIdentity::Anonymous {
                label: "remote-host".into(),
            },
        );
        assert!(session.add_player_with_id(PeerId(20), NetworkPlayerId(20)));
    }
    app.world_mut().resource_mut::<LocalPlayers>().peer = Some(PeerId(20));

    app.world_mut().resource_mut::<LocalServerConfig>().enabled = true;
    app.update();

    let session = app.world().resource::<NetworkSession>();
    assert!(session.owns_player(PeerId(20), NetworkPlayerId(20)));
    assert!(session.owns_player(PeerId(0), NetworkPlayerId(1)));
    assert_eq!(app.world().resource::<LocalPlayers>().peer, Some(PeerId(0)));
}

#[test]
fn local_server_registers_local_players_and_accepts_current_frame_commands() {
    let mut app = unit_app();
    app.add_plugins((AfterglowInputPlugin, AfterglowNetworkPlugin))
        .insert_resource(LocalServerConfig::single_player());
    app.world_mut()
        .resource_mut::<VirtualInputState>()
        .press_action("use");

    app.update();

    let session = app.world().resource::<NetworkSession>();
    assert_eq!(app.world().resource::<LocalPlayers>().peer, Some(PeerId(0)));
    assert_eq!(
        session.peer(PeerId(0)).unwrap().platform,
        PlatformIdentity::Local
    );
    assert!(session.owns_player(PeerId(0), NetworkPlayerId(1)));
    let accepted = app.world().resource::<ServerCommandBuffer>().accepted();
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].peer, PeerId(0));
    assert_eq!(accepted[0].command.player, NetworkPlayerId(1));
    assert_eq!(accepted[0].command.tick, 0);
    assert_eq!(
        accepted[0].command.actions,
        [InputActionValue::pressed("use")]
    );
}

#[test]
fn local_server_keeps_custom_local_players_authoritative() {
    let mut app = unit_app();
    app.add_plugins((AfterglowInputPlugin, AfterglowNetworkPlugin))
        .insert_resource(LocalServerConfig::single_player())
        .insert_resource(LocalPlayers::single(NetworkPlayerId(7)));

    app.update();

    let session = app.world().resource::<NetworkSession>();
    assert!(session.owns_player(PeerId(0), NetworkPlayerId(7)));
    assert!(!session.owns_player(PeerId(0), NetworkPlayerId(1)));
    let accepted = app.world().resource::<ServerCommandBuffer>().accepted();
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].command.player, NetworkPlayerId(7));
}

#[test]
fn local_server_removes_players_no_longer_controlled_locally() {
    let mut app = unit_app();
    app.add_plugins((AfterglowInputPlugin, AfterglowNetworkPlugin))
        .insert_resource(LocalServerConfig::single_player());
    app.update();
    app.world_mut()
        .resource_mut::<LocalPlayers>()
        .remove_player(NetworkPlayerId(1));

    app.update();

    let session = app.world().resource::<NetworkSession>();
    assert!(session.peer(PeerId(0)).is_some());
    assert!(!session.owns_player(PeerId(0), NetworkPlayerId(1)));
    assert!(
        app.world()
            .resource::<ServerCommandBuffer>()
            .accepted()
            .is_empty()
    );
}

#[test]
fn local_server_rejects_commands_for_players_owned_by_another_peer() {
    let mut app = unit_app();
    app.add_plugins((AfterglowInputPlugin, AfterglowNetworkPlugin))
        .insert_resource(LocalServerConfig::single_player())
        .insert_resource(LocalPlayers::single(NetworkPlayerId(7)));
    {
        let mut session = app.world_mut().resource_mut::<NetworkSession>();
        session.connect_peer(
            PeerId(9),
            PlatformIdentity::Anonymous {
                label: "remote".into(),
            },
        );
        assert!(session.add_player_with_id(PeerId(9), NetworkPlayerId(7)));
    }

    app.update();

    let authority = app.world().resource::<ServerCommandBuffer>();
    assert!(authority.accepted().is_empty());
    assert_eq!(authority.rejected().len(), 1);
    assert_eq!(
        authority.rejected()[0].reason,
        CommandRejectReason::PlayerNotOwned
    );
    assert!(
        app.world()
            .resource::<NetworkSession>()
            .owns_player(PeerId(9), NetworkPlayerId(7))
    );
}
