use afterglow_engine::network::PeerId;
use mock_rpg_network_tests::{ClientMsg, Entity, MockClient, MockServer, Player, ServerMsg, Vec3i};

fn joined_world() -> (MockServer, MockClient, MockClient) {
    let mut server = MockServer::new(PeerId(0));
    let mut alice = MockClient::new(PeerId(1), Player(1));
    let mut bob = MockClient::new(PeerId(2), Player(2));
    server.connect(&mut alice);
    server.connect(&mut bob);
    alice.send(
        server.peer,
        ClientMsg::Join {
            player: alice.player,
        },
    );
    bob.send(server.peer, ClientMsg::Join { player: bob.player });
    server.pump(&mut [&mut alice, &mut bob]);
    alice.received.clear();
    bob.received.clear();
    (server, alice, bob)
}

#[test]
fn client_cannot_claim_unassigned_player_id() {
    let mut server = MockServer::new(PeerId(0));
    let mut alice = MockClient::new(PeerId(1), Player(2));
    server.connect(&mut alice);

    alice.send(server.peer, ClientMsg::Join { player: Player(2) });
    server.pump(&mut [&mut alice]);

    assert!(server.replicated.players.is_empty());
    assert!(alice.received.iter().any(
        |msg| matches!(msg, ServerMsg::Reject { reason, .. } if reason == "player-id-mismatch")
    ));
}

#[test]
fn client_cannot_control_another_players_commands() {
    let (mut server, mut alice, mut bob) = joined_world();

    bob.send(
        server.peer,
        ClientMsg::Move {
            player: Player(1),
            tick: 1,
            position: Vec3i::new(32, 0, 0),
        },
    );
    server.pump(&mut [&mut alice, &mut bob]);

    assert_eq!(server.replicated.players[&Player(1)].position, Vec3i::ZERO);
    assert!(bob.received.iter().any(
        |msg| matches!(msg, ServerMsg::Reject { reason, .. } if reason == "player-not-owned")
    ));
}

#[test]
fn client_cannot_teleport_across_the_world() {
    let (mut server, mut alice, mut bob) = joined_world();

    alice.send(
        server.peer,
        ClientMsg::Move {
            player: Player(1),
            tick: 1,
            position: Vec3i::new(i32::MAX, 0, i32::MAX),
        },
    );
    server.pump(&mut [&mut alice, &mut bob]);

    assert_eq!(server.replicated.players[&Player(1)].position, Vec3i::ZERO);
    assert!(
        alice
            .received
            .iter()
            .any(|msg| matches!(msg, ServerMsg::Reject { reason, .. } if reason == "invalid-move"))
    );
}

#[test]
fn client_cannot_interact_with_out_of_reach_entities() {
    let (mut server, mut alice, mut bob) = joined_world();
    server.add_item(Entity(250), Vec3i::new(80, 0, 0));

    alice.send(
        server.peer,
        ClientMsg::Use {
            player: Player(1),
            tick: 1,
            entity: Entity(250),
        },
    );
    server.pump(&mut [&mut alice, &mut bob]);

    assert!(server.item_exists(Entity(250)));
    assert!(
        alice
            .received
            .iter()
            .any(|msg| matches!(msg, ServerMsg::Reject { reason, .. } if reason == "out-of-range"))
    );

    alice.send(
        server.peer,
        ClientMsg::Use {
            player: Player(1),
            tick: 2,
            entity: Entity(999),
        },
    );
    server.pump(&mut [&mut alice, &mut bob]);
    assert!(
        alice.received.iter().any(
            |msg| matches!(msg, ServerMsg::Reject { reason, .. } if reason == "missing-entity")
        )
    );
}

#[test]
fn client_cannot_use_out_of_reach_door() {
    let (mut server, mut alice, mut bob) = joined_world();

    alice.send(
        server.peer,
        ClientMsg::Move {
            player: Player(1),
            tick: 1,
            position: Vec3i::new(20, 0, 20),
        },
    );
    server.pump(&mut [&mut alice, &mut bob]);
    alice.received.clear();

    alice.send(
        server.peer,
        ClientMsg::Use {
            player: Player(1),
            tick: 2,
            entity: Entity(100),
        },
    );
    server.pump(&mut [&mut alice, &mut bob]);

    assert_eq!(server.door_open(Entity(100)), Some(false));
    assert!(
        alice
            .received
            .iter()
            .any(|msg| matches!(msg, ServerMsg::Reject { reason, .. } if reason == "out-of-range"))
    );
}

#[test]
fn client_cannot_attack_out_of_reach_or_missing_npcs() {
    let (mut server, mut alice, mut bob) = joined_world();

    alice.send(
        server.peer,
        ClientMsg::Attack {
            player: Player(1),
            tick: 1,
            entity: Entity(300),
        },
    );
    server.pump(&mut [&mut alice, &mut bob]);

    assert_eq!(server.npc_hp(Entity(300)), Some(10));
    assert!(
        alice
            .received
            .iter()
            .any(|msg| matches!(msg, ServerMsg::Reject { reason, .. } if reason == "out-of-range"))
    );
    alice.received.clear();

    alice.send(
        server.peer,
        ClientMsg::Attack {
            player: Player(1),
            tick: 2,
            entity: Entity(999),
        },
    );
    server.pump(&mut [&mut alice, &mut bob]);

    assert!(
        alice.received.iter().any(
            |msg| matches!(msg, ServerMsg::Reject { reason, .. } if reason == "missing-entity")
        )
    );
}

#[test]
fn malformed_packets_are_ignored_without_disconnect_or_panic() {
    let (mut server, mut alice, mut bob) = joined_world();

    alice.send_raw(server.peer, b"{not json".to_vec());
    server.pump(&mut [&mut alice, &mut bob]);

    assert!(alice.received.is_empty());
    assert!(bob.received.is_empty());
}
