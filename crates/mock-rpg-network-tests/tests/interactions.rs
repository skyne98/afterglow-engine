use afterglow_engine::network::PeerId;
use mock_rpg_network_tests::{
    Chunk, ClientMsg, Entity, MockClient, MockServer, Player, ServerMsg, Vec3i, WorldEvent,
};

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
    (server, alice, bob)
}

#[test]
fn clients_join_and_receive_authoritative_spawn_snapshot() {
    let (_server, alice, bob) = joined_world();

    assert!(matches!(
        alice.received.as_slice(),
        [
            ServerMsg::Welcome {
                player: Player(1),
                chunk: Chunk(0, 0, 0)
            },
            ..
        ]
    ));
    assert!(matches!(
        bob.received.as_slice(),
        [
            ServerMsg::Welcome {
                player: Player(2),
                chunk: Chunk(0, 0, 0)
            },
            ..
        ]
    ));
    assert!(
        alice
            .received
            .iter()
            .any(|msg| snapshot_has_player(msg, Player(1)))
    );
    assert!(
        bob.received
            .iter()
            .any(|msg| snapshot_has_player(msg, Player(2)))
    );
}

#[test]
fn chunk_interest_limits_visible_snapshots() {
    let (mut server, mut alice, mut bob) = joined_world();
    alice.received.clear();
    bob.received.clear();

    alice.send(
        server.peer,
        ClientMsg::Move {
            player: Player(1),
            tick: 1,
            position: Vec3i::new(128, 0, 0),
        },
    );
    server.pump(&mut [&mut alice, &mut bob]);

    assert!(
        alice
            .received
            .iter()
            .any(|msg| snapshot_has_player(msg, Player(1)))
    );
    assert!(
        !alice
            .received
            .iter()
            .any(|msg| snapshot_has_player(msg, Player(2)))
    );
}

#[test]
fn nearby_door_use_replicates_to_interested_clients() {
    let (mut server, mut alice, mut bob) = joined_world();
    alice.received.clear();
    bob.received.clear();

    alice.send(
        server.peer,
        ClientMsg::Use {
            player: Player(1),
            tick: 1,
            entity: Entity(100),
        },
    );
    server.pump(&mut [&mut alice, &mut bob]);

    let opened = ServerMsg::Event(WorldEvent::DoorOpened(Entity(100)));
    assert!(alice.received.contains(&opened));
    assert!(bob.received.contains(&opened));
}

#[test]
fn item_pickup_is_authoritative_and_conflict_safe() {
    let (mut server, mut alice, mut bob) = joined_world();
    alice.received.clear();
    bob.received.clear();

    alice.send(
        server.peer,
        ClientMsg::Use {
            player: Player(1),
            tick: 1,
            entity: Entity(200),
        },
    );
    bob.send(
        server.peer,
        ClientMsg::Use {
            player: Player(2),
            tick: 1,
            entity: Entity(200),
        },
    );
    server.pump(&mut [&mut alice, &mut bob]);

    let picked_by_alice = ServerMsg::Event(WorldEvent::ItemPickedUp {
        entity: Entity(200),
        by: Player(1),
    });
    assert!(alice.received.contains(&picked_by_alice));
    assert!(bob.received.contains(&picked_by_alice));
    assert!(
        bob.received.iter().any(
            |msg| matches!(msg, ServerMsg::Reject { reason, .. } if reason == "missing-entity")
        )
    );
    assert!(server.players[&Player(1)].inventory.contains(&Entity(200)));
    assert!(!server.players[&Player(2)].inventory.contains(&Entity(200)));
}

#[test]
fn combat_only_affects_entities_inside_player_interest_space() {
    let (mut server, mut alice, mut bob) = joined_world();
    alice.received.clear();
    bob.received.clear();

    alice.send(
        server.peer,
        ClientMsg::Move {
            player: Player(1),
            tick: 1,
            position: Vec3i::new(36, 0, 4),
        },
    );
    server.pump(&mut [&mut alice, &mut bob]);
    alice.received.clear();
    bob.received.clear();

    alice.send(
        server.peer,
        ClientMsg::Attack {
            player: Player(1),
            tick: 2,
            entity: Entity(300),
        },
    );
    server.pump(&mut [&mut alice, &mut bob]);

    let damaged = ServerMsg::Event(WorldEvent::NpcDamaged {
        entity: Entity(300),
        hp: 7,
    });
    assert!(alice.received.contains(&damaged));
    assert!(bob.received.contains(&damaged));
}

#[test]
fn duplicate_command_ticks_are_idempotent() {
    let (mut server, mut alice, mut bob) = joined_world();
    alice.received.clear();
    bob.received.clear();

    alice.send(
        server.peer,
        ClientMsg::Move {
            player: Player(1),
            tick: 1,
            position: Vec3i::new(36, 0, 4),
        },
    );
    server.pump(&mut [&mut alice, &mut bob]);
    alice.received.clear();

    alice.send(
        server.peer,
        ClientMsg::Attack {
            player: Player(1),
            tick: 2,
            entity: Entity(300),
        },
    );
    alice.send(
        server.peer,
        ClientMsg::Attack {
            player: Player(1),
            tick: 2,
            entity: Entity(300),
        },
    );
    server.pump(&mut [&mut alice, &mut bob]);

    let damage_events = alice
        .received
        .iter()
        .filter(|msg| matches!(msg, ServerMsg::Event(WorldEvent::NpcDamaged { .. })))
        .count();
    assert_eq!(damage_events, 1);
}

fn snapshot_has_player(msg: &ServerMsg, player: Player) -> bool {
    match msg {
        ServerMsg::Snapshot { players, .. } => players.iter().any(|(id, _, _)| *id == player),
        _ => false,
    }
}
