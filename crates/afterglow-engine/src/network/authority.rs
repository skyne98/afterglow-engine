use crate::{
    input::PlayerCommand,
    network::{NetworkPlayerId, PeerId, session::NetworkSession},
};
use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Resource, Clone, Debug, Default, PartialEq, Reflect)]
pub struct ServerCommandBuffer {
    accepted: Vec<AuthoritativePlayerCommand>,
    rejected: Vec<RejectedPlayerCommand>,
    seen_ticks: BTreeMap<NetworkPlayerId, BTreeSet<u32>>,
}

#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct AuthoritativePlayerCommand {
    pub peer: PeerId,
    pub command: PlayerCommand,
}

#[derive(Clone, Debug, Eq, PartialEq, Reflect)]
pub struct RejectedPlayerCommand {
    pub peer: PeerId,
    pub player: NetworkPlayerId,
    pub tick: u32,
    pub reason: CommandRejectReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub enum CommandRejectReason {
    UnknownPlayer,
    PlayerNotOwned,
    DuplicateTick,
}

impl ServerCommandBuffer {
    pub fn begin_frame(&mut self) {
        self.accepted.clear();
        self.rejected.clear();
    }

    pub fn submit(
        &mut self,
        peer: PeerId,
        command: PlayerCommand,
        session: &NetworkSession,
    ) -> CommandAuthorityResult {
        let player = command.player;
        let tick = command.tick;
        let result = self.validate(peer, player, tick, session);
        match result {
            CommandAuthorityResult::Accepted => {
                self.accepted
                    .push(AuthoritativePlayerCommand { peer, command });
            }
            CommandAuthorityResult::Rejected(reason) => {
                self.rejected.push(RejectedPlayerCommand {
                    peer,
                    player,
                    tick,
                    reason,
                });
            }
        }
        result
    }

    pub fn submit_many(
        &mut self,
        peer: PeerId,
        commands: impl IntoIterator<Item = PlayerCommand>,
        session: &NetworkSession,
    ) {
        for command in commands {
            self.submit(peer, command, session);
        }
    }

    pub fn accepted(&self) -> &[AuthoritativePlayerCommand] {
        &self.accepted
    }

    pub fn rejected(&self) -> &[RejectedPlayerCommand] {
        &self.rejected
    }

    pub fn forget_player(&mut self, player: NetworkPlayerId) {
        self.seen_ticks.remove(&player);
    }

    fn validate(
        &mut self,
        peer: PeerId,
        player: NetworkPlayerId,
        tick: u32,
        session: &NetworkSession,
    ) -> CommandAuthorityResult {
        let Some(player_session) = session.player(player) else {
            return CommandAuthorityResult::Rejected(CommandRejectReason::UnknownPlayer);
        };
        if player_session.peer != peer {
            return CommandAuthorityResult::Rejected(CommandRejectReason::PlayerNotOwned);
        }
        if !self.seen_ticks.entry(player).or_default().insert(tick) {
            return CommandAuthorityResult::Rejected(CommandRejectReason::DuplicateTick);
        }
        CommandAuthorityResult::Accepted
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandAuthorityResult {
    Accepted,
    Rejected(CommandRejectReason),
}

pub fn clear_server_command_buffer(mut commands: ResMut<ServerCommandBuffer>) {
    commands.begin_frame();
}

#[cfg(test)]
mod tests;
