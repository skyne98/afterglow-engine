use afterglow_engine::{
    input::PlayerCommand,
    network::{
        DeliveryMode, MemoryTransport, NetChannel, NetworkTransport, PeerId, TransportEvent,
        authority::{CommandAuthorityResult, ServerCommandBuffer},
        session::{NetworkSession, PlatformIdentity},
    },
};
use serde::{Deserialize, Serialize};

mod replicated;
mod rules;
pub use replicated::*;
use rules::*;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Player(pub u64);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Chunk(pub i32, pub i32, pub i32);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Entity(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Vec3i {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl Vec3i {
    pub const ZERO: Self = Self::new(0, 0, 0);

    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    pub fn distance_squared(self, other: Self) -> i64 {
        let dx = i64::from(self.x) - i64::from(other.x);
        let dy = i64::from(self.y) - i64::from(other.y);
        let dz = i64::from(self.z) - i64::from(other.z);
        dx * dx + dy * dy + dz * dz
    }

    pub fn chunk(self) -> Chunk {
        Chunk(
            self.x.div_euclid(32),
            self.y.div_euclid(16),
            self.z.div_euclid(32),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ClientMsg {
    Join {
        player: Player,
    },
    Move {
        player: Player,
        tick: u32,
        position: Vec3i,
    },
    Use {
        player: Player,
        tick: u32,
        entity: Entity,
    },
    Attack {
        player: Player,
        tick: u32,
        entity: Entity,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ServerMsg {
    Welcome {
        player: Player,
        chunk: Chunk,
    },
    Snapshot {
        tick: u32,
        players: Vec<(Player, Vec3i, i32)>,
        doors: Vec<(Entity, bool)>,
        items: Vec<Entity>,
        npcs: Vec<(Entity, Vec3i, i32)>,
    },
    Event(WorldEvent),
    Reject {
        tick: u32,
        reason: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum WorldEvent {
    DoorOpened(Entity),
    ItemPickedUp { entity: Entity, by: Player },
    NpcDamaged { entity: Entity, hp: i32 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorldState {
    session: NetworkSession,
    commands: ServerCommandBuffer,
    replicated: ReplicatedWorld,
    replicated_messages: Vec<ReplicatedMessage>,
    tick: u32,
}

#[derive(Debug)]
pub struct MockClient {
    pub peer: PeerId,
    pub player: Player,
    pub transport: MemoryTransport,
    pub received: Vec<ServerMsg>,
}

#[derive(Debug)]
pub struct MockServer {
    pub peer: PeerId,
    pub transport: MemoryTransport,
    session: NetworkSession,
    commands: ServerCommandBuffer,
    pub replicated: ReplicatedWorld,
    replicated_messages: Vec<ReplicatedMessage>,
    tick: u32,
}

impl MockClient {
    pub fn new(peer: PeerId, player: Player) -> Self {
        Self {
            peer,
            player,
            transport: MemoryTransport::new(peer),
            received: Vec::new(),
        }
    }

    pub fn send(&mut self, server: PeerId, msg: ClientMsg) {
        self.send_raw(server, serde_json::to_vec(&msg).unwrap());
    }

    pub fn send_raw(&mut self, server: PeerId, payload: Vec<u8>) {
        self.transport.send(
            server,
            NetChannel::Commands,
            DeliveryMode::UnreliableSequenced,
            payload,
        );
    }

    pub fn poll(&mut self) {
        let mut events = Vec::new();
        self.transport.poll_events(&mut events);
        self.received.extend(events.into_iter().filter_map(|event| {
            let TransportEvent::Packet(packet) = event else {
                return None;
            };
            serde_json::from_slice(&packet.payload).ok()
        }));
    }
}

impl MockServer {
    pub fn new(peer: PeerId) -> Self {
        Self {
            peer,
            transport: MemoryTransport::new(peer),
            session: NetworkSession::default(),
            commands: ServerCommandBuffer::default(),
            replicated: ReplicatedWorld::default(),
            replicated_messages: Vec::new(),
            tick: 0,
        }
    }

    pub fn connect(&mut self, client: &mut MockClient) {
        self.transport.connect_peer(client.peer);
        client.transport.connect_peer(self.peer);
        self.session.connect_peer(
            client.peer,
            PlatformIdentity::Anonymous {
                label: format!("peer-{}", client.peer.0),
            },
        );
    }

    pub fn add_npc(&mut self, entity: Entity, position: Vec3i, hp: i32) {
        self.replicated.npcs.insert(entity, (position, hp));
    }

    pub fn add_item(&mut self, entity: Entity, position: Vec3i) {
        self.replicated.items.insert(entity, position);
    }

    pub fn save_state(&self) -> WorldState {
        WorldState {
            session: self.session.clone(),
            commands: self.commands.clone(),
            replicated: self.replicated.clone(),
            replicated_messages: self.replicated_messages.clone(),
            tick: self.tick,
        }
    }

    pub fn restore_state(&mut self, state: WorldState) {
        self.session = state.session;
        self.commands = state.commands;
        self.replicated = state.replicated;
        self.replicated_messages = state.replicated_messages;
        self.tick = state.tick;
    }

    pub fn npc_hp(&self, entity: Entity) -> Option<i32> {
        self.replicated.npcs.get(&entity).map(|(_, hp)| *hp)
    }

    pub fn door_open(&self, entity: Entity) -> Option<bool> {
        self.replicated.doors.get(&entity).map(|(_, open)| *open)
    }

    pub fn item_exists(&self, entity: Entity) -> bool {
        self.replicated.items.contains_key(&entity)
    }

    pub fn pump(&mut self, clients: &mut [&mut MockClient]) {
        let mut server_incoming = Vec::new();
        for client in clients.iter_mut() {
            server_incoming.extend(client.transport.collect_outgoing());
        }
        self.transport.receive_packets(server_incoming);
        self.transport.advance_tick();
        self.commands.begin_frame();
        self.handle_events();

        let server_outgoing = self.transport.collect_outgoing();
        for client in clients.iter_mut() {
            let packets = server_outgoing
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

    fn handle_events(&mut self) {
        let mut events = Vec::new();
        self.transport.poll_events(&mut events);
        for event in events {
            let TransportEvent::Packet(packet) = event else {
                continue;
            };
            if let Ok(msg) = serde_json::from_slice::<ClientMsg>(&packet.payload) {
                self.apply(packet.from, msg);
            }
        }
    }

    fn apply(&mut self, peer: PeerId, msg: ClientMsg) {
        match msg {
            ClientMsg::Join { player } => self.join(peer, player),
            ClientMsg::Move {
                player,
                tick,
                position,
            } => {
                if !self.accept_command(peer, player, tick) {
                    return;
                }
                if valid_move(self.player(peer).position, position) {
                    self.emit_replicated(ReplicatedMessage::PlayerMoved { player, position });
                    self.send_snapshot(peer);
                } else {
                    self.reject(peer, tick, "invalid-move");
                }
            }
            ClientMsg::Use {
                player,
                tick,
                entity,
            } => {
                if self.accept_command(peer, player, tick) {
                    self.use_entity(peer, player, tick, entity);
                }
            }
            ClientMsg::Attack {
                player,
                tick,
                entity,
            } => {
                if self.accept_command(peer, player, tick) {
                    self.attack_entity(peer, tick, entity);
                }
            }
        }
    }

    fn join(&mut self, peer: PeerId, player: Player) {
        let network_player = net_player(player);
        if self
            .replicated
            .players
            .get(&player)
            .is_some_and(|state| state.peer != peer)
        {
            self.reject(peer, 0, "player-already-owned");
            return;
        }
        if let Some(session_player) = self.session.player(network_player) {
            if session_player.peer != peer {
                self.reject(peer, 0, "player-already-owned");
                return;
            }
        } else {
            let mut session = self.session.clone();
            let assigned = session.add_player(peer);
            if assigned != Some(network_player) {
                self.reject(peer, 0, "player-id-mismatch");
                return;
            }
            self.session = session;
        }
        if !self.replicated.players.contains_key(&player) {
            self.emit_replicated(ReplicatedMessage::PlayerJoined { player, peer });
        }
        self.send(
            peer,
            ServerMsg::Welcome {
                player,
                chunk: Vec3i::ZERO.chunk(),
            },
        );
        self.send_snapshot(peer);
    }

    fn use_entity(&mut self, peer: PeerId, player: Player, tick: u32, entity: Entity) {
        let position = self.player(peer).position;
        if let Some((door_pos, open)) = self.replicated.doors.get(&entity).copied() {
            if in_reach(position, door_pos) {
                if !open {
                    self.emit_replicated(ReplicatedMessage::DoorOpened { entity });
                }
                self.broadcast_near(
                    door_pos.chunk(),
                    ServerMsg::Event(WorldEvent::DoorOpened(entity)),
                );
                return;
            }
            self.reject(peer, tick, "out-of-range");
            return;
        }
        if let Some(item_pos) = self.replicated.items.get(&entity).copied() {
            if in_reach(position, item_pos) {
                self.emit_replicated(ReplicatedMessage::ItemPickedUp { entity, by: player });
                self.broadcast_near(
                    item_pos.chunk(),
                    ServerMsg::Event(WorldEvent::ItemPickedUp { entity, by: player }),
                );
            } else {
                self.reject(peer, tick, "out-of-range");
            }
        } else {
            self.reject(peer, tick, "missing-entity");
        }
    }

    fn attack_entity(&mut self, peer: PeerId, tick: u32, entity: Entity) {
        let position = self.player(peer).position;
        if let Some((npc_pos, old_hp)) = self.replicated.npcs.get(&entity).copied() {
            if in_reach(position, npc_pos) {
                let chunk = npc_pos.chunk();
                let hp = (old_hp - 3).max(0);
                self.emit_replicated(ReplicatedMessage::NpcDamaged { entity, hp });
                self.broadcast_near(
                    chunk,
                    ServerMsg::Event(WorldEvent::NpcDamaged { entity, hp }),
                );
            } else {
                self.reject(peer, tick, "out-of-range");
            }
        } else {
            self.reject(peer, tick, "missing-entity");
        }
    }

    fn accept_command(&mut self, peer: PeerId, player: Player, tick: u32) -> bool {
        let result = self.commands.submit(
            peer,
            PlayerCommand {
                player: net_player(player),
                tick,
                ..Default::default()
            },
            &self.session,
        );
        match result {
            CommandAuthorityResult::Accepted => true,
            CommandAuthorityResult::Rejected(reason) => {
                self.reject(peer, tick, reject_reason(reason));
                false
            }
        }
    }

    fn emit_replicated(&mut self, message: ReplicatedMessage) {
        self.replicated_messages.push(message.clone());
        self.replicated.apply_message(message);
    }

    fn send_snapshot(&mut self, peer: PeerId) {
        let chunk = self.player(peer).position.chunk();
        self.send(peer, self.snapshot_for(chunk));
    }

    fn snapshot_for(&self, center: Chunk) -> ServerMsg {
        ServerMsg::Snapshot {
            tick: self.tick,
            players: self
                .replicated
                .players
                .iter()
                .filter(|(_, state)| near(center, state.position.chunk()))
                .map(|(player, state)| (*player, state.position, state.hp))
                .collect(),
            doors: self
                .replicated
                .doors
                .iter()
                .filter(|(_, (position, _))| near(center, position.chunk()))
                .map(|(entity, (_, open))| (*entity, *open))
                .collect(),
            items: self
                .replicated
                .items
                .iter()
                .filter(|(_, position)| near(center, position.chunk()))
                .map(|(entity, _)| *entity)
                .collect(),
            npcs: self
                .replicated
                .npcs
                .iter()
                .filter(|(_, (position, _))| near(center, position.chunk()))
                .map(|(entity, (position, hp))| (*entity, *position, *hp))
                .collect(),
        }
    }

    fn broadcast_near(&mut self, chunk: Chunk, msg: ServerMsg) {
        let peers = self
            .replicated
            .players
            .values()
            .filter(|state| near(chunk, state.position.chunk()))
            .map(|state| state.peer)
            .collect::<Vec<_>>();
        for peer in peers {
            self.send(peer, msg.clone());
        }
    }

    fn reject(&mut self, peer: PeerId, tick: u32, reason: &str) {
        self.send(
            peer,
            ServerMsg::Reject {
                tick,
                reason: reason.into(),
            },
        );
    }

    fn send(&mut self, peer: PeerId, msg: ServerMsg) {
        self.transport.send(
            peer,
            NetChannel::Snapshots,
            DeliveryMode::Reliable,
            serde_json::to_vec(&msg).unwrap(),
        );
    }

    fn player(&self, peer: PeerId) -> &PlayerState {
        self.replicated
            .players
            .values()
            .find(|state| state.peer == peer)
            .unwrap()
    }
}
