use super::{ReplicationAppExt, ReplicationRegistration};
use crate::network::rollback::{
    RollbackCommit, RollbackMessage, RollbackMessageDiff, RollbackMessageStream, RollbackPolicy,
};
use bevy::prelude::*;

#[derive(Resource, Clone, Copy, Debug, Default, Eq, PartialEq, Reflect)]
pub struct RollbackReplicationClock {
    pub current_tick: u32,
    pub policy: RollbackPolicy,
}

#[derive(Resource, Clone, Debug, Eq, PartialEq, Reflect)]
pub struct ReplicatedRollbackMessageStream<E> {
    messages: RollbackMessageStream<E>,
    last_diff: RollbackMessageDiff<E>,
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

impl<E> Default for ReplicatedRollbackMessageStream<E> {
    fn default() -> Self {
        Self {
            messages: RollbackMessageStream::default(),
            last_diff: RollbackMessageDiff {
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

impl<E> ReplicatedRollbackMessageStream<E> {
    pub fn messages(&self) -> &RollbackMessageStream<E> {
        &self.messages
    }

    pub fn last_diff(&self) -> &RollbackMessageDiff<E> {
        &self.last_diff
    }

    pub fn last_commit(&self) -> &RollbackCommit<E> {
        &self.last_commit
    }
}

impl<E> ReplicatedRollbackMessageStream<E>
where
    E: Clone + PartialEq,
{
    pub fn replace_provisional(
        &mut self,
        messages: impl IntoIterator<Item = RollbackMessage<E>>,
    ) -> &RollbackMessageDiff<E> {
        self.last_diff = self.messages.replace_provisional(messages);
        &self.last_diff
    }

    pub fn commit_through(&mut self, committed_tick: u32) -> &RollbackCommit<E> {
        self.last_commit = self.messages.commit_through(committed_tick);
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
