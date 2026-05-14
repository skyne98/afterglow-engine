use super::math::{Vec3f, segment_distance_squared};
use afterglow_engine::{
    core::identity::StableEntityId,
    input::{InputActionValue, PlayerCommand},
    network::{
        DeliveryMode, FaultConfig, MemoryTransport, NetChannel, NetworkPlayerId, NetworkTransport,
        PeerId, TransportEvent,
        authority::{CommandAuthorityResult, ServerCommandBuffer},
        interpolation::{RemoteEntitySample, RemoteInterpolationBuffer, SmoothingMode},
        prediction::ClientPredictionBuffer,
        reconciliation::ClientReconciliationQueue,
        replication::WorldSnapshot,
        session::{NetworkSession, PlatformIdentity},
    },
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const SERVER: PeerId = PeerId(0);
const ALICE: NetworkPlayerId = NetworkPlayerId(1);
const BOB: NetworkPlayerId = NetworkPlayerId(2);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum ClientMsg {
    Move {
        player: u64,
        tick: u32,
        position: Vec3f,
    },
    Shoot {
        player: u64,
        tick: u32,
        origin: Vec3f,
        velocity: Vec3f,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum ServerMsg {
    Snapshot {
        tick: u32,
        players: Vec<(u64, Vec3f, i32)>,
        projectiles: Vec<(u64, u64, Vec3f)>,
    },
    Hit {
        tick: u32,
        projectile: u64,
        target: u64,
        hp: i32,
    },
    Reject {
        tick: u32,
        reason: String,
    },
}

#[derive(Clone, Copy, Debug)]
struct Body {
    position: Vec3f,
    radius: f32,
    hp: i32,
}

#[derive(Clone, Copy, Debug)]
struct Projectile {
    id: u64,
    owner: NetworkPlayerId,
    previous: Vec3f,
    position: Vec3f,
    velocity: Vec3f,
    radius: f32,
    alive: bool,
}

struct SpellServer {
    tick: u32,
    transport: MemoryTransport,
    session: NetworkSession,
    commands: ServerCommandBuffer,
    players: BTreeMap<NetworkPlayerId, Body>,
    projectiles: Vec<Projectile>,
    next_projectile: u64,
}

struct SpellClient {
    peer: PeerId,
    player: NetworkPlayerId,
    transport: MemoryTransport,
    prediction: ClientPredictionBuffer,
    reconciliation: ClientReconciliationQueue,
    smoothing: RemoteInterpolationBuffer,
    hits: Vec<(u32, u64, NetworkPlayerId, i32)>,
    seen_projectiles: BTreeSet<u64>,
}

impl SpellServer {
    fn new() -> Self {
        let mut session = NetworkSession::default();
        session.connect_peer(PeerId(1), PlatformIdentity::Local);
        session.connect_peer(PeerId(2), PlatformIdentity::Local);
        assert_eq!(session.add_player(PeerId(1)), Some(ALICE));
        assert_eq!(session.add_player(PeerId(2)), Some(BOB));
        let mut transport = MemoryTransport::new(SERVER).with_faults(FaultConfig {
            delay_ticks: 1,
            reverse_delivery: true,
            ..Default::default()
        });
        transport.connect_peer(PeerId(1));
        transport.connect_peer(PeerId(2));
        Self {
            tick: 0,
            transport,
            session,
            commands: ServerCommandBuffer::default(),
            players: BTreeMap::from([
                (ALICE, Body::new(Vec3f::new(-4.0, 0.0, 0.0))),
                (BOB, Body::new(Vec3f::new(4.0, 0.0, 0.0))),
            ]),
            projectiles: Vec::new(),
            next_projectile: 1,
        }
    }

    fn pump(&mut self, clients: &mut [&mut SpellClient]) {
        let mut incoming = Vec::new();
        for client in clients.iter_mut() {
            incoming.extend(client.transport.collect_outgoing());
        }
        self.transport.receive_packets(incoming);
        self.transport.advance_tick();
        self.commands.begin_frame();
        self.handle_packets();
        self.step_projectiles();
        self.broadcast_snapshot();

        let outgoing = self.transport.collect_outgoing();
        for client in clients {
            let packets = outgoing
                .iter()
                .filter(|packet| packet.to == client.peer)
                .cloned()
                .collect();
            client.transport.receive_packets(packets);
            client.transport.advance_tick();
            client.poll();
        }
        self.tick = self.tick.saturating_add(1);
    }

    fn handle_packets(&mut self) {
        let mut events = Vec::new();
        self.transport.poll_events(&mut events);
        for event in events {
            let TransportEvent::Packet(packet) = event else {
                continue;
            };
            let Ok(msg) = serde_json::from_slice::<ClientMsg>(&packet.payload) else {
                continue;
            };
            self.apply(packet.from, msg);
        }
    }

    fn apply(&mut self, peer: PeerId, msg: ClientMsg) {
        match msg {
            ClientMsg::Move {
                player,
                tick,
                position,
            } if self.accept(peer, NetworkPlayerId(player), tick, "move") => {
                if let Some(body) = self.players.get_mut(&NetworkPlayerId(player)) {
                    body.position = position;
                }
            }
            ClientMsg::Shoot {
                player,
                tick,
                origin,
                velocity,
            } if self.accept(peer, NetworkPlayerId(player), tick, "cast_spell") => {
                self.spawn_projectile(NetworkPlayerId(player), origin, velocity);
            }
            ClientMsg::Move { .. } | ClientMsg::Shoot { .. } => {}
        }
    }

    fn accept(&mut self, peer: PeerId, player: NetworkPlayerId, tick: u32, action: &str) -> bool {
        let result = self.commands.submit(
            peer,
            PlayerCommand {
                player,
                tick,
                actions: vec![InputActionValue::pressed(action)],
                ..Default::default()
            },
            &self.session,
        );
        match result {
            CommandAuthorityResult::Accepted => true,
            CommandAuthorityResult::Rejected(reason) => {
                self.send(
                    peer,
                    ServerMsg::Reject {
                        tick,
                        reason: format!("{reason:?}"),
                    },
                );
                false
            }
        }
    }

    fn spawn_projectile(&mut self, owner: NetworkPlayerId, origin: Vec3f, velocity: Vec3f) {
        self.projectiles.push(Projectile {
            id: self.next_projectile,
            owner,
            previous: origin,
            position: origin,
            velocity,
            radius: 0.25,
            alive: true,
        });
        self.next_projectile += 1;
    }

    fn step_projectiles(&mut self) {
        let mut hits = Vec::new();
        for projectile in &mut self.projectiles {
            if !projectile.alive {
                continue;
            }
            projectile.previous = projectile.position;
            projectile.position = projectile.position.add(projectile.velocity);
            for (player, body) in &mut self.players {
                if *player == projectile.owner {
                    continue;
                }
                let radius = projectile.radius + body.radius;
                if segment_distance_squared(projectile.previous, projectile.position, body.position)
                    <= radius * radius
                {
                    body.hp -= 25;
                    projectile.alive = false;
                    hits.push((projectile.id, *player, body.hp));
                    break;
                }
            }
        }
        self.projectiles.retain(|projectile| projectile.alive);
        for (projectile, target, hp) in hits {
            self.broadcast(ServerMsg::Hit {
                tick: self.tick,
                projectile,
                target: target.0,
                hp,
            });
        }
    }

    fn broadcast_snapshot(&mut self) {
        let players = self
            .players
            .iter()
            .map(|(player, body)| (player.0, body.position, body.hp))
            .collect();
        let projectiles = self
            .projectiles
            .iter()
            .map(|projectile| (projectile.id, projectile.owner.0, projectile.position))
            .collect();
        self.broadcast(ServerMsg::Snapshot {
            tick: self.tick,
            players,
            projectiles,
        });
    }

    fn broadcast(&mut self, msg: ServerMsg) {
        for peer in [PeerId(1), PeerId(2)] {
            self.send(peer, msg.clone());
        }
    }

    fn send(&mut self, peer: PeerId, msg: ServerMsg) {
        self.transport.send(
            peer,
            NetChannel::Snapshots,
            DeliveryMode::Reliable,
            serde_json::to_vec(&msg).unwrap(),
        );
    }
}

impl Body {
    fn new(position: Vec3f) -> Self {
        Self {
            position,
            radius: 0.75,
            hp: 100,
        }
    }
}

impl SpellClient {
    fn new(peer: PeerId, player: NetworkPlayerId) -> Self {
        let mut transport = MemoryTransport::new(peer).with_faults(FaultConfig {
            delay_ticks: 2,
            reverse_delivery: true,
            ..Default::default()
        });
        transport.connect_peer(SERVER);
        Self {
            peer,
            player,
            transport,
            prediction: ClientPredictionBuffer::default(),
            reconciliation: ClientReconciliationQueue::default(),
            smoothing: RemoteInterpolationBuffer::default().with_timing(2, 1),
            hits: Vec::new(),
            seen_projectiles: BTreeSet::new(),
        }
    }

    fn move_to(&mut self, tick: u32, position: Vec3f) {
        self.send(ClientMsg::Move {
            player: self.player.0,
            tick,
            position,
        });
    }

    fn shoot(&mut self, tick: u32, origin: Vec3f, velocity: Vec3f) {
        self.prediction.record(PlayerCommand {
            player: self.player,
            tick,
            actions: vec![InputActionValue::pressed("cast_spell")],
            ..Default::default()
        });
        self.send(ClientMsg::Shoot {
            player: self.player.0,
            tick,
            origin,
            velocity,
        });
    }

    fn send(&mut self, msg: ClientMsg) {
        self.transport.send(
            SERVER,
            NetChannel::Commands,
            DeliveryMode::UnreliableSequenced,
            serde_json::to_vec(&msg).unwrap(),
        );
    }

    fn poll(&mut self) {
        let mut events = Vec::new();
        self.transport.poll_events(&mut events);
        for event in events {
            let TransportEvent::Packet(packet) = event else {
                continue;
            };
            let Ok(msg) = serde_json::from_slice::<ServerMsg>(&packet.payload) else {
                continue;
            };
            self.handle(msg);
        }
    }

    fn handle(&mut self, msg: ServerMsg) {
        match msg {
            ServerMsg::Snapshot {
                tick,
                players,
                projectiles,
            } => {
                self.reconciliation.reconcile_snapshot(
                    &mut self.prediction,
                    self.player,
                    &WorldSnapshot {
                        tick,
                        entities: Vec::new(),
                    },
                );
                for (player, position, hp) in players {
                    self.smoothing.record(
                        player_entity(NetworkPlayerId(player)),
                        tick,
                        sample(position).with_field("hp", hp as f32),
                    );
                }
                for (projectile, owner, position) in projectiles {
                    self.seen_projectiles.insert(projectile);
                    self.smoothing.record(
                        projectile_entity(projectile),
                        tick,
                        sample(position).with_field("owner", owner as f32),
                    );
                }
            }
            ServerMsg::Hit {
                tick,
                projectile,
                target,
                hp,
            } => self
                .hits
                .push((tick, projectile, NetworkPlayerId(target), hp)),
            ServerMsg::Reject { .. } => {}
        }
    }
}

pub fn moving_players_exchange_spell_projectiles_over_delayed_reordered_network() {
    let mut server = SpellServer::new();
    let mut alice = SpellClient::new(PeerId(1), ALICE);
    let mut bob = SpellClient::new(PeerId(2), BOB);

    alice.move_to(1, Vec3f::new(-4.0, 0.0, 0.0));
    bob.move_to(1, Vec3f::new(4.0, 0.0, 0.0));
    alice.shoot(2, Vec3f::new(-4.0, 0.0, 0.0), Vec3f::new(1.25, 0.0, 0.0));
    bob.shoot(2, Vec3f::new(4.0, 0.0, 0.0), Vec3f::new(-1.25, 0.0, 0.0));

    assert_eq!(alice.prediction.pending_len(ALICE), 1);
    assert_eq!(bob.prediction.pending_len(BOB), 1);

    for tick in 0..12 {
        if tick == 3 {
            alice.move_to(3, Vec3f::new(-3.5, 0.0, 0.0));
            bob.move_to(3, Vec3f::new(3.5, 0.0, 0.0));
        }
        server.pump(&mut [&mut alice, &mut bob]);
    }

    assert_eq!(server.players[&ALICE].hp, 75);
    assert_eq!(server.players[&BOB].hp, 75);
    assert!(
        alice
            .hits
            .iter()
            .any(|(_, _, target, hp)| *target == ALICE && *hp == 75)
    );
    assert!(
        alice
            .hits
            .iter()
            .any(|(_, _, target, hp)| *target == BOB && *hp == 75)
    );
    assert!(
        bob.hits
            .iter()
            .any(|(_, _, target, hp)| *target == ALICE && *hp == 75)
    );
    assert!(
        bob.hits
            .iter()
            .any(|(_, _, target, hp)| *target == BOB && *hp == 75)
    );

    assert_eq!(alice.prediction.pending_len(ALICE), 0);
    assert_eq!(bob.prediction.pending_len(BOB), 0);
    assert!(alice.seen_projectiles.len() >= 2);
    assert!(bob.seen_projectiles.len() >= 2);

    let bob_player = alice
        .smoothing
        .sample_at(player_entity(BOB), 4.5)
        .expect("bob should have buffered player samples");
    assert_eq!(bob_player.mode, SmoothingMode::Interpolated);

    let projectile_id = *alice.seen_projectiles.iter().next().unwrap();
    let projectile = alice
        .smoothing
        .sample_at(projectile_entity(projectile_id), 3.5)
        .expect("projectile should interpolate between delayed snapshots");
    assert_eq!(projectile.mode, SmoothingMode::Interpolated);

    assert!(
        alice
            .smoothing
            .sample_at(projectile_entity(projectile_id), 20.0)
            .is_none()
    );
}

fn player_entity(player: NetworkPlayerId) -> StableEntityId {
    StableEntityId::from_raw(1_000 + player.0 as u128)
}

fn projectile_entity(projectile: u64) -> StableEntityId {
    StableEntityId::from_raw(10_000 + projectile as u128)
}

fn sample(position: Vec3f) -> RemoteEntitySample {
    RemoteEntitySample::default()
        .with_field("pos_x", position.x)
        .with_field("pos_y", position.y)
        .with_field("pos_z", position.z)
}
