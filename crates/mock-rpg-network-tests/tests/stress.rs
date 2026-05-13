use afterglow_engine::network::{FaultConfig, MemoryTransport, PeerId};
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

#[test]
fn many_peers_spam_commands_under_unstable_transport_without_state_corruption() {
    const PEERS: usize = 512;
    const NPCS: usize = 5 * 1024;
    const GROUPS: usize = 64;
    const ROUNDS: usize = 16;

    let mut server = MockServer::new(PeerId(0));
    server.transport = MemoryTransport::new(PeerId(0)).with_faults(FaultConfig {
        drop_every: Some(17),
        duplicate_every: Some(5),
        delay_ticks: 2,
        reverse_delivery: false,
    });
    let mut clients = (0..PEERS)
        .map(|index| {
            let peer = PeerId(index as u64 + 1);
            let mut client = MockClient::new(peer, Player(index as u64 + 1));
            client.transport = MemoryTransport::new(peer).with_faults(FaultConfig {
                drop_every: Some(23),
                duplicate_every: Some(7),
                delay_ticks: (index % 3) as u32,
                reverse_delivery: index % 2 == 0,
            });
            client
        })
        .collect::<Vec<_>>();

    for client in &mut clients {
        server.connect(client);
        client.send(
            server.peer,
            ClientMsg::Join {
                player: client.player,
            },
        );
    }
    for _ in 0..4 {
        pump_all(&mut server, &mut clients);
    }
    assert_eq!(server.replicated.players.len(), PEERS);
    for (client_index, client) in clients.iter().enumerate() {
        let state = server.replicated.players.get_mut(&client.player).unwrap();
        state.position = group_origin(client_index % GROUPS);
    }

    for index in 0..NPCS {
        let origin = group_origin(index % GROUPS);
        let local = index / GROUPS;
        server.add_npc(
            Entity(10_000 + index as u64),
            Vec3i::new(
                origin.x + (local % 4) as i32,
                0,
                origin.z + ((local / 4) % 4) as i32,
            ),
            60,
        );
    }

    for round in 0..ROUNDS {
        for (client_index, client) in clients.iter_mut().enumerate() {
            let player = client.player;
            let group = client_index % GROUPS;
            let local = client_index / GROUPS;
            let origin = group_origin(group);
            let base_tick = (round as u32 * 4) + 1;
            let target_local = (round * (PEERS / GROUPS) + local) % (NPCS / GROUPS);
            let target = Entity(10_000 + (group + target_local * GROUPS) as u64);
            client.send(
                server.peer,
                ClientMsg::Move {
                    player,
                    tick: base_tick,
                    position: origin,
                },
            );
            client.send(
                server.peer,
                ClientMsg::Attack {
                    player,
                    tick: base_tick + 1,
                    entity: target,
                },
            );
            client.send(
                server.peer,
                ClientMsg::Attack {
                    player,
                    tick: base_tick + 1,
                    entity: target,
                },
            );
            let msg = if round % 5 == 0 {
                ClientMsg::Move {
                    player,
                    tick: base_tick + 2,
                    position: Vec3i::new(origin.x + 10_000, 0, origin.z),
                }
            } else {
                ClientMsg::Use {
                    player,
                    tick: base_tick + 2,
                    entity: Entity(100),
                }
            };
            client.send(server.peer, msg);
        }
        pump_all(&mut server, &mut clients);
    }
    for _ in 0..8 {
        pump_all(&mut server, &mut clients);
    }

    assert_eq!(server.replicated.players.len(), PEERS);
    assert_eq!(server.replicated.npcs.len(), NPCS + 1);
    assert!(server.door_open(Entity(100)).unwrap_or(false));
    let damaged_npcs = (0..NPCS)
        .filter(|index| server.npc_hp(Entity(10_000 + *index as u64)).unwrap() < 60)
        .count();
    assert!(damaged_npcs >= NPCS / 2);
    for index in 0..NPCS {
        let hp = server.npc_hp(Entity(10_000 + index as u64)).unwrap();
        assert!((0..=60).contains(&hp));
    }
}

fn damage_event_count(messages: &[ServerMsg]) -> usize {
    messages
        .iter()
        .filter(|msg| matches!(msg, ServerMsg::Event(WorldEvent::NpcDamaged { .. })))
        .count()
}

fn pump_all(server: &mut MockServer, clients: &mut [MockClient]) {
    let mut client_refs = clients.iter_mut().collect::<Vec<_>>();
    server.pump(&mut client_refs);
}

fn group_origin(group: usize) -> Vec3i {
    Vec3i::new((group % 8) as i32 * 96, 0, (group / 8) as i32 * 96)
}
