use std::collections::HashMap;

use crate::{
    input::{PlayerCommand, PlayerCommandQueue},
    network::NetworkPlayerId,
};

pub(crate) struct PlayerCommandLookup<'a> {
    latest: HashMap<NetworkPlayerId, &'a PlayerCommand>,
}

impl<'a> PlayerCommandLookup<'a> {
    pub(crate) fn new(queue: &'a PlayerCommandQueue) -> Self {
        let mut latest = HashMap::with_capacity(queue.commands().len());
        for command in queue.commands() {
            latest.insert(command.player, command);
        }
        Self { latest }
    }

    pub(crate) fn get(&self, player: NetworkPlayerId) -> Option<&'a PlayerCommand> {
        self.latest.get(&player).copied()
    }
}
