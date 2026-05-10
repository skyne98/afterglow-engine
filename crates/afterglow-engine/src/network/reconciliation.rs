use crate::{
    input::PlayerCommand,
    network::{
        NetworkPlayerId,
        prediction::ClientPredictionBuffer,
        replication::{WorldDelta, WorldSnapshot},
    },
};
use bevy::prelude::*;

#[derive(Resource, Clone, Debug, Default, PartialEq, Reflect)]
pub struct ClientReconciliationQueue {
    results: Vec<ReconciliationResult>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub enum AuthoritativeUpdateSource {
    Snapshot,
    Delta,
    Correction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub struct AuthoritativeCorrection {
    pub player: NetworkPlayerId,
    pub tick: u32,
    pub source: AuthoritativeUpdateSource,
}

#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct ReconciliationResult {
    pub player: NetworkPlayerId,
    pub authoritative_tick: u32,
    pub source: AuthoritativeUpdateSource,
    pub replay_commands: Vec<PlayerCommand>,
}

impl ClientReconciliationQueue {
    pub fn reconcile(
        &mut self,
        prediction: &mut ClientPredictionBuffer,
        correction: AuthoritativeCorrection,
    ) -> &ReconciliationResult {
        self.results.push(reconcile(prediction, correction));
        self.results.last().unwrap()
    }

    pub fn reconcile_snapshot(
        &mut self,
        prediction: &mut ClientPredictionBuffer,
        player: NetworkPlayerId,
        snapshot: &WorldSnapshot,
    ) -> &ReconciliationResult {
        self.reconcile(
            prediction,
            AuthoritativeCorrection {
                player,
                tick: snapshot.tick,
                source: AuthoritativeUpdateSource::Snapshot,
            },
        )
    }

    pub fn reconcile_delta(
        &mut self,
        prediction: &mut ClientPredictionBuffer,
        player: NetworkPlayerId,
        delta: &WorldDelta,
    ) -> &ReconciliationResult {
        self.reconcile(
            prediction,
            AuthoritativeCorrection {
                player,
                tick: delta.to_tick,
                source: AuthoritativeUpdateSource::Delta,
            },
        )
    }

    pub fn results(&self) -> &[ReconciliationResult] {
        &self.results
    }

    pub fn clear(&mut self) {
        self.results.clear();
    }
}

pub fn reconcile(
    prediction: &mut ClientPredictionBuffer,
    correction: AuthoritativeCorrection,
) -> ReconciliationResult {
    let replay = prediction.replay_after(correction.player, correction.tick);
    ReconciliationResult {
        player: correction.player,
        authoritative_tick: correction.tick,
        source: correction.source,
        replay_commands: replay.commands,
    }
}

pub fn clear_reconciliation_queue(mut queue: ResMut<ClientReconciliationQueue>) {
    queue.clear();
}

#[cfg(test)]
mod tests;
