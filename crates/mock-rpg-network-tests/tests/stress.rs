use afterglow_engine::network::PeerId;
use mock_rpg_network_tests::{
    ClientMsg, Entity, MockClient, MockServer, Player, ServerMsg, Vec3i, WorldEvent,
};

#[test]
fn many_npcs_and_world_changes_can_resolve_in_one_server_tick() {
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

    alice.send(
        server.peer,
        ClientMsg::Move {
            player: Player(1),
            tick: 1,
            position: Vec3i::new(32, 0, 0),
        },
    );
    bob.send(
        server.peer,
        ClientMsg::Move {
            player: Player(2),
            tick: 1,
            position: Vec3i::new(34, 0, 0),
        },
    );
    server.pump(&mut [&mut alice, &mut bob]);

    for index in 0..64 {
        server.add_npc(
            Entity(1_000 + index),
            Vec3i::new(32 + (index as i32 % 4), 0, 0),
            10,
        );
    }
    alice.received.clear();
    bob.received.clear();

    for index in 0..32 {
        alice.send(
            server.peer,
            ClientMsg::Attack {
                player: Player(1),
                tick: 10 + index as u32,
                entity: Entity(1_000 + index),
            },
        );
    }
    for index in 32..64 {
        bob.send(
            server.peer,
            ClientMsg::Attack {
                player: Player(2),
                tick: 10 + index as u32,
                entity: Entity(1_000 + index),
            },
        );
    }
    server.pump(&mut [&mut alice, &mut bob]);

    for index in 0..64 {
        assert_eq!(server.npc_hp(Entity(1_000 + index)), Some(7));
    }
    let alice_damage_events = damage_event_count(&alice.received);
    let bob_damage_events = damage_event_count(&bob.received);
    assert!(alice_damage_events >= 32);
    assert!(bob_damage_events >= 32);
}

#[test]
fn snapshots_can_contain_many_simultaneous_world_states() {
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

    for index in 0..96 {
        server.add_npc(
            Entity(2_000 + index),
            Vec3i::new(index as i32 % 16, 0, index as i32 / 16),
            5,
        );
    }
    alice.received.clear();
    alice.send(
        server.peer,
        ClientMsg::Move {
            player: Player(1),
            tick: 1,
            position: Vec3i::new(1, 0, 1),
        },
    );
    server.pump(&mut [&mut alice]);

    let npc_count = alice
        .received
        .iter()
        .filter_map(|msg| match msg {
            ServerMsg::Snapshot { npcs, .. } => Some(npcs.len()),
            _ => None,
        })
        .max()
        .unwrap_or_default();
    assert!(npc_count >= 96);
}

fn damage_event_count(messages: &[ServerMsg]) -> usize {
    messages
        .iter()
        .filter(|msg| matches!(msg, ServerMsg::Event(WorldEvent::NpcDamaged { .. })))
        .count()
}
