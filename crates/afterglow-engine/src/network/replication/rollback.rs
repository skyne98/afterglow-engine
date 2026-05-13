use super::{ReplicationAppExt, ReplicationRegistration};
use crate::network::rollback::{
    RollbackCommit, RollbackEvent, RollbackEventDiff, RollbackEventStream, RollbackPolicy,
};
use bevy::prelude::*;

#[derive(Resource, Clone, Copy, Debug, Default, Eq, PartialEq, Reflect)]
pub struct RollbackReplicationClock {
    pub current_tick: u32,
    pub policy: RollbackPolicy,
}

#[derive(Resource, Clone, Debug, Eq, PartialEq, Reflect)]
pub struct ReplicatedRollbackEventStream<E> {
    events: RollbackEventStream<E>,
    last_diff: RollbackEventDiff<E>,
    last_commit: RollbackCommit<E>,
}

impl RollbackReplicationClock {
    pub fn new(current_tick: u32, policy: RollbackPolicy) -> Self {
        Self {
            current_tick,
            policy,
        }
    }

    pub fn committed_tick(&self) -> u32 {
        self.policy.committed_tick(self.current_tick)
    }
}

impl<E> Default for ReplicatedRollbackEventStream<E> {
    fn default() -> Self {
        Self {
            events: RollbackEventStream::default(),
            last_diff: RollbackEventDiff {
                added: Vec::new(),
                removed: Vec::new(),
            },
            last_commit: RollbackCommit {
                committed_tick: 0,
                added: Vec::new(),
            },
        }
    }
}

impl<E> ReplicatedRollbackEventStream<E> {
    pub fn events(&self) -> &RollbackEventStream<E> {
        &self.events
    }

    pub fn last_diff(&self) -> &RollbackEventDiff<E> {
        &self.last_diff
    }

    pub fn last_commit(&self) -> &RollbackCommit<E> {
        &self.last_commit
    }
}

impl<E> ReplicatedRollbackEventStream<E>
where
    E: Clone + PartialEq,
{
    pub fn replace_provisional(
        &mut self,
        events: impl IntoIterator<Item = RollbackEvent<E>>,
    ) -> &RollbackEventDiff<E> {
        self.last_diff = self.events.replace_provisional(events);
        &self.last_diff
    }

    pub fn commit_through(&mut self, committed_tick: u32) -> &RollbackCommit<E> {
        self.last_commit = self.events.commit_through(committed_tick);
        &self.last_commit
    }
}

impl ReplicationAppExt for App {
    fn replicate<R>(&mut self, registration: R) -> &mut Self
    where
        R: ReplicationRegistration,
    {
        registration.register(self);
        self
    }
}
