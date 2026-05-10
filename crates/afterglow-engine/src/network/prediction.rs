use crate::{
    input::PlayerCommand,
    network::{NetworkPlayerId, replication::WorldSnapshot},
};
use bevy::prelude::*;
use std::collections::BTreeMap;

#[derive(Resource, Clone, Debug, Default, PartialEq, Reflect)]
pub struct ClientPredictionBuffer {
    commands: BTreeMap<NetworkPlayerId, BTreeMap<u32, PlayerCommand>>,
    acknowledged: BTreeMap<NetworkPlayerId, u32>,
}

#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct PredictionReplay {
    pub player: NetworkPlayerId,
    pub authoritative_tick: u32,
    pub commands: Vec<PlayerCommand>,
}

impl ClientPredictionBuffer {
    pub fn record(&mut self, command: PlayerCommand) -> Option<PlayerCommand> {
        self.commands
            .entry(command.player)
            .or_default()
            .insert(command.tick, command)
    }

    pub fn acknowledge(&mut self, player: NetworkPlayerId, tick: u32) {
        self.acknowledged
            .entry(player)
            .and_modify(|ack| *ack = (*ack).max(tick))
            .or_insert(tick);
        if let Some(commands) = self.commands.get_mut(&player) {
            commands.retain(|command_tick, _| *command_tick > tick);
            if commands.is_empty() {
                self.commands.remove(&player);
            }
        }
    }

    pub fn acknowledge_snapshot(&mut self, player: NetworkPlayerId, snapshot: &WorldSnapshot) {
        self.acknowledge(player, snapshot.tick);
    }

    pub fn replay_after(
        &mut self,
        player: NetworkPlayerId,
        authoritative_tick: u32,
    ) -> PredictionReplay {
        self.acknowledge(player, authoritative_tick);
        PredictionReplay {
            player,
            authoritative_tick,
            commands: self.pending(player).cloned().collect(),
        }
    }

    pub fn pending(
        &self,
        player: NetworkPlayerId,
    ) -> impl DoubleEndedIterator<Item = &PlayerCommand> {
        self.commands
            .get(&player)
            .into_iter()
            .flat_map(|commands| commands.values())
    }

    pub fn acknowledged_tick(&self, player: NetworkPlayerId) -> Option<u32> {
        self.acknowledged.get(&player).copied()
    }

    pub fn pending_len(&self, player: NetworkPlayerId) -> usize {
        self.commands.get(&player).map_or(0, BTreeMap::len)
    }

    pub fn clear_player(&mut self, player: NetworkPlayerId) {
        self.commands.remove(&player);
        self.acknowledged.remove(&player);
    }
}

#[cfg(test)]
mod tests;
