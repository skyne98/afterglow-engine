use crate::{Entity, Player, Vec3i};
use afterglow_engine::network::PeerId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplicatedEvent {
    PlayerJoined { player: Player, peer: PeerId },
    PlayerMoved { player: Player, position: Vec3i },
    DoorOpened { entity: Entity },
    ItemPickedUp { entity: Entity, by: Player },
    NpcDamaged { entity: Entity, hp: i32 },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlayerState {
    pub peer: PeerId,
    pub position: Vec3i,
    pub hp: i32,
    pub inventory: BTreeSet<Entity>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReplicatedWorld {
    pub players: BTreeMap<Player, PlayerState>,
    pub doors: BTreeMap<Entity, (Vec3i, bool)>,
    pub items: BTreeMap<Entity, Vec3i>,
    pub npcs: BTreeMap<Entity, (Vec3i, i32)>,
}

impl Default for ReplicatedWorld {
    fn default() -> Self {
        Self {
            players: BTreeMap::new(),
            doors: BTreeMap::from([(Entity(100), (Vec3i::new(4, 0, 4), false))]),
            items: BTreeMap::from([(Entity(200), Vec3i::new(5, 0, 5))]),
            npcs: BTreeMap::from([(Entity(300), (Vec3i::new(36, 0, 4), 10))]),
        }
    }
}

impl ReplicatedWorld {
    pub fn apply_event(&mut self, event: ReplicatedEvent) {
        match event {
            ReplicatedEvent::PlayerJoined { player, peer } => {
                self.players.entry(player).or_insert(PlayerState {
                    peer,
                    position: Vec3i::ZERO,
                    hp: 100,
                    inventory: BTreeSet::new(),
                });
            }
            ReplicatedEvent::PlayerMoved { player, position } => {
                if let Some(state) = self.players.get_mut(&player) {
                    state.position = position;
                }
            }
            ReplicatedEvent::DoorOpened { entity } => {
                if let Some((_, open)) = self.doors.get_mut(&entity) {
                    *open = true;
                }
            }
            ReplicatedEvent::ItemPickedUp { entity, by } => {
                self.items.remove(&entity);
                if let Some(player) = self.players.get_mut(&by) {
                    player.inventory.insert(entity);
                }
            }
            ReplicatedEvent::NpcDamaged { entity, hp } => {
                if let Some((_, npc_hp)) = self.npcs.get_mut(&entity) {
                    *npc_hp = hp;
                }
            }
        }
    }
}
