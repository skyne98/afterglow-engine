use afterglow_engine::network::PeerId;
use mock_rpg_network_tests::{
    ClientMsg, Entity, MockClient, MockServer, Player, ServerMsg, Vec3i, WorldEvent,
};

#[test]
fn server_world_state_can_roll_back_and_replay_authoritative_commands() {
    let mut server = MockServer::new(PeerId(0));
    let mut alice = MockClient::new(PeerId(1), Player(1));
    server.connect(&mut alice);
    alice.send(
        server.peer,
        ClientMsg::Join {
            player: alice.player,
        },
    );
    server.pump(&mut [&mut alice]);

    alice.send(
        server.peer,
        ClientMsg::Move {
            player: Player(1),
            tick: 1,
            position: Vec3i::new(36, 0, 4),
        },
    );
    server.pump(&mut [&mut alice]);
    let before_attack = server.save_state();

    alice.send(
        server.peer,
        ClientMsg::Attack {
            player: Player(1),
            tick: 2,
            entity: Entity(300),
        },
    );
    server.pump(&mut [&mut alice]);
    assert_eq!(server.npc_hp(Entity(300)), Some(7));

    server.restore_state(before_attack);
    assert_eq!(server.npc_hp(Entity(300)), Some(10));

    alice.send(
        server.peer,
        ClientMsg::Attack {
            player: Player(1),
            tick: 2,
            entity: Entity(300),
        },
    );
    server.pump(&mut [&mut alice]);
    assert_eq!(server.npc_hp(Entity(300)), Some(7));
}

#[test]
fn restored_world_replays_interaction_visibility_consistently() {
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
    let before_use = server.save_state();

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
    let first_events = alice.received.clone();
    assert_eq!(server.door_open(Entity(100)), Some(true));

    server.restore_state(before_use);
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

    assert_eq!(server.door_open(Entity(100)), Some(true));
    assert_eq!(
        first_events
            .into_iter()
            .filter(|msg| matches!(msg, ServerMsg::Event(WorldEvent::DoorOpened(_))))
            .collect::<Vec<_>>(),
        alice
            .received
            .iter()
            .filter(|msg| matches!(msg, ServerMsg::Event(WorldEvent::DoorOpened(_))))
            .cloned()
            .collect::<Vec<_>>()
    );
}
