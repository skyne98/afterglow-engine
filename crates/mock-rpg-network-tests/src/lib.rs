use afterglow_engine::network::{
    DeliveryMode, MemoryTransport, NetChannel, NetworkTransport, PeerId, TransportEvent,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

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

    pub fn distance_squared(self, other: Self) -> i32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        let dz = self.z - other.z;
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlayerState {
    pub peer: PeerId,
    pub position: Vec3i,
    pub hp: i32,
    pub inventory: BTreeSet<Entity>,
    seen_ticks: BTreeSet<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldState {
    players: BTreeMap<Player, PlayerState>,
    doors: BTreeMap<Entity, (Vec3i, bool)>,
    items: BTreeMap<Entity, Vec3i>,
    npcs: BTreeMap<Entity, (Vec3i, i32)>,
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
    pub players: BTreeMap<Player, PlayerState>,
    doors: BTreeMap<Entity, (Vec3i, bool)>,
    items: BTreeMap<Entity, Vec3i>,
    npcs: BTreeMap<Entity, (Vec3i, i32)>,
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
            players: BTreeMap::new(),
            doors: BTreeMap::from([(Entity(100), (Vec3i::new(4, 0, 4), false))]),
            items: BTreeMap::from([(Entity(200), Vec3i::new(5, 0, 5))]),
            npcs: BTreeMap::from([(Entity(300), (Vec3i::new(36, 0, 4), 10))]),
            tick: 0,
        }
    }

    pub fn connect(&mut self, client: &mut MockClient) {
        self.transport.connect_peer(client.peer);
        client.transport.connect_peer(self.peer);
    }

    pub fn add_npc(&mut self, entity: Entity, position: Vec3i, hp: i32) {
        self.npcs.insert(entity, (position, hp));
    }

    pub fn add_item(&mut self, entity: Entity, position: Vec3i) {
        self.items.insert(entity, position);
    }

    pub fn save_state(&self) -> WorldState {
        WorldState {
            players: self.players.clone(),
            doors: self.doors.clone(),
            items: self.items.clone(),
            npcs: self.npcs.clone(),
            tick: self.tick,
        }
    }

    pub fn restore_state(&mut self, state: WorldState) {
        self.players = state.players;
        self.doors = state.doors;
        self.items = state.items;
        self.npcs = state.npcs;
        self.tick = state.tick;
    }

    pub fn npc_hp(&self, entity: Entity) -> Option<i32> {
        self.npcs.get(&entity).map(|(_, hp)| *hp)
    }

    pub fn door_open(&self, entity: Entity) -> Option<bool> {
        self.doors.get(&entity).map(|(_, open)| *open)
    }

    pub fn item_exists(&self, entity: Entity) -> bool {
        self.items.contains_key(&entity)
    }

    pub fn pump(&mut self, clients: &mut [&mut MockClient]) {
        let mut server_incoming = Vec::new();
        for client in clients.iter_mut() {
            server_incoming.extend(client.transport.collect_outgoing());
        }
        self.transport.receive_packets(server_incoming);
        self.transport.advance_tick();
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
                if !self.owns_player(peer, player) {
                    self.reject(peer, tick, "player-not-owned");
                } else if self.accept_tick(player, tick) && valid_move(self.player(peer), position)
                {
                    self.players.get_mut(&player).unwrap().position = position;
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
                if !self.owns_player(peer, player) {
                    self.reject(peer, tick, "player-not-owned");
                } else if self.accept_tick(player, tick) {
                    self.use_entity(peer, player, tick, entity);
                }
            }
            ClientMsg::Attack {
                player,
                tick,
                entity,
            } => {
                if !self.owns_player(peer, player) {
                    self.reject(peer, tick, "player-not-owned");
                } else if self.accept_tick(player, tick) {
                    self.attack_entity(peer, entity);
                }
            }
        }
    }

    fn join(&mut self, peer: PeerId, player: Player) {
        if self
            .players
            .get(&player)
            .is_some_and(|state| state.peer != peer)
        {
            self.reject(peer, 0, "player-already-owned");
            return;
        }
        self.players.entry(player).or_insert(PlayerState {
            peer,
            position: Vec3i::ZERO,
            hp: 100,
            inventory: BTreeSet::new(),
            seen_ticks: BTreeSet::new(),
        });
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
        if let Some((door_pos, open)) = self.doors.get_mut(&entity) {
            if in_reach(position, *door_pos) {
                let door_pos = *door_pos;
                *open = true;
                self.broadcast_near(
                    door_pos.chunk(),
                    ServerMsg::Event(WorldEvent::DoorOpened(entity)),
                );
                return;
            }
        }
        if let Some(item_pos) = self.items.remove(&entity) {
            if in_reach(position, item_pos) {
                self.players
                    .get_mut(&player)
                    .unwrap()
                    .inventory
                    .insert(entity);
                self.broadcast_near(
                    item_pos.chunk(),
                    ServerMsg::Event(WorldEvent::ItemPickedUp { entity, by: player }),
                );
            } else {
                self.items.insert(entity, item_pos);
                self.reject(peer, tick, "out-of-range");
            }
        } else {
            self.reject(peer, tick, "missing-entity");
        }
    }

    fn attack_entity(&mut self, peer: PeerId, entity: Entity) {
        let position = self.player(peer).position;
        if let Some((npc_pos, hp)) = self.npcs.get_mut(&entity) {
            if in_reach(position, *npc_pos) {
                let chunk = npc_pos.chunk();
                *hp = (*hp - 3).max(0);
                let hp = *hp;
                self.broadcast_near(
                    chunk,
                    ServerMsg::Event(WorldEvent::NpcDamaged { entity, hp }),
                );
            }
        }
    }

    fn owns_player(&self, peer: PeerId, player: Player) -> bool {
        self.players
            .get(&player)
            .is_some_and(|state| state.peer == peer)
    }

    fn accept_tick(&mut self, player: Player, tick: u32) -> bool {
        self.players
            .get_mut(&player)
            .is_some_and(|state| state.seen_ticks.insert(tick))
    }

    fn send_snapshot(&mut self, peer: PeerId) {
        let chunk = self.player(peer).position.chunk();
        self.send(peer, self.snapshot_for(chunk));
    }

    fn snapshot_for(&self, center: Chunk) -> ServerMsg {
        ServerMsg::Snapshot {
            tick: self.tick,
            players: self
                .players
                .iter()
                .filter(|(_, state)| near(center, state.position.chunk()))
                .map(|(player, state)| (*player, state.position, state.hp))
                .collect(),
            doors: self
                .doors
                .iter()
                .filter(|(_, (position, _))| near(center, position.chunk()))
                .map(|(entity, (_, open))| (*entity, *open))
                .collect(),
            items: self
                .items
                .iter()
                .filter(|(_, position)| near(center, position.chunk()))
                .map(|(entity, _)| *entity)
                .collect(),
            npcs: self
                .npcs
                .iter()
                .filter(|(_, (position, _))| near(center, position.chunk()))
                .map(|(entity, (position, hp))| (*entity, *position, *hp))
                .collect(),
        }
    }

    fn broadcast_near(&mut self, chunk: Chunk, msg: ServerMsg) {
        let peers = self
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
        self.players
            .values()
            .find(|state| state.peer == peer)
            .unwrap()
    }
}

fn valid_move(player: &PlayerState, target: Vec3i) -> bool {
    player.position.distance_squared(target) <= 160 * 160
}

fn in_reach(a: Vec3i, b: Vec3i) -> bool {
    a.distance_squared(b) <= 8 * 8
}

fn near(a: Chunk, b: Chunk) -> bool {
    (a.0 - b.0).abs() <= 1 && (a.1 - b.1).abs() <= 1 && (a.2 - b.2).abs() <= 1
}
