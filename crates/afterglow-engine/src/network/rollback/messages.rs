use super::RollbackDomainId;
use crate::core::identity::StableEntityId;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    Reflect,
    Serialize,
    Deserialize,
)]
pub struct RollbackMessageId {
    pub domain: RollbackDomainId,
    pub tick: u32,
    pub sequence: u64,
}

impl RollbackMessageId {
    pub const fn new(domain: RollbackDomainId, tick: u32, sequence: u64) -> Self {
        Self {
            domain,
            tick,
            sequence,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Reflect, Serialize, Deserialize)]
pub struct RollbackMessage<T> {
    pub id: RollbackMessageId,
    pub source_command_tick: Option<u32>,
    pub entities: Vec<StableEntityId>,
    pub payload: T,
}

impl<T> Message for RollbackMessage<T> where T: Send + Sync + 'static {}

impl<T> RollbackMessage<T> {
    pub fn new(id: RollbackMessageId, payload: T) -> Self {
        Self {
            id,
            source_command_tick: None,
            entities: Vec::new(),
            payload,
        }
    }

    pub fn with_source_command_tick(mut self, tick: u32) -> Self {
        self.source_command_tick = Some(tick);
        self
    }

    pub fn with_entities(mut self, entities: impl IntoIterator<Item = StableEntityId>) -> Self {
        self.entities = entities.into_iter().collect();
        self
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Reflect)]
pub struct RollbackMessageDiff<T> {
    pub added: Vec<RollbackMessage<T>>,
    pub removed: Vec<RollbackMessageId>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Reflect)]
pub struct RollbackCommit<T> {
    pub committed_tick: u32,
    pub added: Vec<RollbackMessage<T>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Reflect)]
pub struct RollbackMessageStream<T> {
    provisional: Vec<RollbackMessageBucket<T>>,
    committed: Vec<RollbackMessageBucket<T>>,
    committed_tick: Option<u32>,
}

impl<T> Default for RollbackMessageStream<T> {
    fn default() -> Self {
        Self {
            provisional: Vec::new(),
            committed: Vec::new(),
            committed_tick: None,
        }
    }
}

impl<T> RollbackMessageStream<T> {
    pub fn provisional(&self) -> impl Iterator<Item = &RollbackMessage<T>> {
        self.provisional
            .iter()
            .flat_map(|bucket| bucket.messages.iter())
    }

    pub fn committed(&self) -> impl Iterator<Item = &RollbackMessage<T>> {
        self.committed
            .iter()
            .flat_map(|bucket| bucket.messages.iter())
    }

    pub fn provisional_message(&self, id: RollbackMessageId) -> Option<&RollbackMessage<T>> {
        message_by_id(&self.provisional, id)
    }

    pub fn committed_message(&self, id: RollbackMessageId) -> Option<&RollbackMessage<T>> {
        message_by_id(&self.committed, id)
    }

    pub fn committed_tick(&self) -> Option<u32> {
        self.committed_tick
    }

    pub fn clear(&mut self) {
        self.provisional.clear();
        self.committed.clear();
        self.committed_tick = None;
    }
}

impl<T> RollbackMessageStream<T>
where
    T: Clone + PartialEq,
{
    /// Replaces the current replay-produced provisional message stream.
    ///
    /// Visual systems consume the returned diff. Gameplay and durable business
    /// systems should not treat provisional removals as manual undo hooks; they
    /// should read the rebuilt provisional state or wait for committed
    /// messages.
    pub fn replace_provisional(
        &mut self,
        messages: impl IntoIterator<Item = RollbackMessage<T>>,
    ) -> RollbackMessageDiff<T> {
        let committed_tick = self.committed_tick;
        let next_messages = canonical_messages(
            messages
                .into_iter()
                .filter(|message| committed_tick.is_none_or(|tick| message.id.tick > tick))
                .collect(),
        );
        let RollbackMessageDiff { added, removed } =
            diff_ordered(self.provisional(), &next_messages);
        let next = buckets_from_sorted(next_messages);

        self.provisional = next;
        RollbackMessageDiff { added, removed }
    }

    /// Promotes provisional messages at or before `committed_tick` into the
    /// final stream.
    ///
    /// Committed consumers run durable business logic from this result: save
    /// data, irreversible quest state, inventory persistence, achievements,
    /// and external IO.
    pub fn commit_through(&mut self, committed_tick: u32) -> RollbackCommit<T> {
        let committed_tick = self
            .committed_tick
            .map_or(committed_tick, |previous| previous.max(committed_tick));
        let mut added = Vec::new();

        let split_at = self
            .provisional
            .partition_point(|bucket| bucket.tick <= committed_tick);
        let ready = self.provisional.drain(..split_at).collect::<Vec<_>>();
        let mut committed_messages = self.committed().cloned().collect::<Vec<_>>();

        for bucket in ready {
            for message in bucket.messages {
                let changed = self.committed_message(message.id) != Some(&message);
                if changed {
                    added.push(message.clone());
                }
                committed_messages.push(message);
            }
        }

        self.committed = buckets_from_sorted(canonical_messages(committed_messages));
        self.committed_tick = Some(committed_tick);

        RollbackCommit {
            committed_tick,
            added,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Reflect)]
struct RollbackMessageBucket<T> {
    tick: u32,
    messages: Vec<RollbackMessage<T>>,
}

fn buckets_from_sorted<T>(messages: Vec<RollbackMessage<T>>) -> Vec<RollbackMessageBucket<T>> {
    let mut buckets = Vec::<RollbackMessageBucket<T>>::new();
    for message in messages {
        let needs_new_bucket = buckets
            .last()
            .is_none_or(|bucket| bucket.tick != message.id.tick);
        if needs_new_bucket {
            buckets.push(RollbackMessageBucket {
                tick: message.id.tick,
                messages: Vec::new(),
            });
        }

        let bucket_index = buckets.len() - 1;
        if buckets[bucket_index]
            .messages
            .last()
            .is_some_and(|previous| previous.id == message.id)
        {
            *buckets[bucket_index].messages.last_mut().unwrap() = message;
            continue;
        }
        buckets[bucket_index].messages.push(message);
    }
    buckets
}

fn canonical_messages<T>(messages: Vec<RollbackMessage<T>>) -> Vec<RollbackMessage<T>> {
    let mut canonical = BTreeMap::new();
    for message in messages {
        canonical.insert(message.id, message);
    }
    canonical.into_values().collect()
}

fn diff_ordered<'a, T>(
    previous: impl Iterator<Item = &'a RollbackMessage<T>>,
    next: &'a [RollbackMessage<T>],
) -> RollbackMessageDiff<T>
where
    T: Clone + PartialEq + 'a,
{
    let mut previous = previous.peekable();
    let mut next_index = 0;
    let mut added = Vec::new();
    let mut removed = Vec::new();

    while let (Some(&previous_message), Some(next_message)) =
        (previous.peek(), next.get(next_index))
    {
        match previous_message.id.cmp(&next_message.id) {
            std::cmp::Ordering::Less => {
                removed.push(previous_message.id);
                previous.next();
            }
            std::cmp::Ordering::Greater => {
                added.push(next_message.clone());
                next_index += 1;
            }
            std::cmp::Ordering::Equal => {
                if previous_message != next_message {
                    removed.push(previous_message.id);
                    added.push(next_message.clone());
                }
                previous.next();
                next_index += 1;
            }
        }
    }

    removed.extend(previous.map(|message| message.id));
    added.extend(next[next_index..].iter().cloned());

    RollbackMessageDiff { added, removed }
}

fn message_by_id<T>(
    buckets: &[RollbackMessageBucket<T>],
    id: RollbackMessageId,
) -> Option<&RollbackMessage<T>> {
    let bucket = buckets
        .binary_search_by_key(&id.tick, |bucket| bucket.tick)
        .ok()
        .and_then(|index| buckets.get(index))?;
    bucket
        .messages
        .binary_search_by_key(&id, |message| message.id)
        .ok()
        .and_then(|index| bucket.messages.get(index))
}

#[cfg(test)]
mod tests;
