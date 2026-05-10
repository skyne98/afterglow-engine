use super::*;

#[test]
fn session_maps_peer_to_platform_player_and_avatar() {
    let mut session = NetworkSession::default();
    let peer = PeerId(10);
    let avatar = StableEntityId::from_raw(99);

    assert!(session.connect_peer(peer, PlatformIdentity::Steam { steam_id: 42 }));
    let player = session.add_player(peer).unwrap();
    let bound = session.bind_avatar(player, avatar);

    assert_eq!(bound, Some(avatar));
    assert_eq!(
        session.peer(peer).unwrap().platform,
        PlatformIdentity::Steam { steam_id: 42 }
    );
    assert_eq!(session.peer(peer).unwrap().players, [player]);
    assert_eq!(session.player(player).unwrap().avatar, Some(avatar));
    assert_eq!(session.player_for_avatar(avatar), Some(player));
    assert!(session.owns_player(peer, player));
}

#[test]
fn one_peer_can_own_multiple_players() {
    let mut session = NetworkSession::default();
    let peer = PeerId(1);

    session.connect_peer(peer, PlatformIdentity::Local);
    let player_a = session.add_player(peer).unwrap();
    let player_b = session.add_player(peer).unwrap();

    assert_ne!(player_a, player_b);
    assert_eq!(session.peer(peer).unwrap().players, [player_a, player_b]);
    assert!(session.owns_player(peer, player_a));
    assert!(session.owns_player(peer, player_b));
}

#[test]
fn disconnect_removes_peer_owned_players() {
    let mut session = NetworkSession::default();
    let peer = PeerId(1);

    session.connect_peer(
        peer,
        PlatformIdentity::Anonymous {
            label: "dev".into(),
        },
    );
    let player = session.add_player(peer).unwrap();

    assert_eq!(session.disconnect_peer(peer), [player]);
    assert!(session.peer(peer).is_none());
    assert!(session.player(player).is_none());
    assert!(!session.owns_player(peer, player));
}

#[test]
fn duplicate_peer_connections_are_rejected() {
    let mut session = NetworkSession::default();

    assert!(session.connect_peer(PeerId(1), PlatformIdentity::Local));
    assert!(!session.connect_peer(PeerId(1), PlatformIdentity::Local));
}
