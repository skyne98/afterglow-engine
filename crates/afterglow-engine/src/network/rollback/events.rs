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
pub struct RollbackEventId {
    pub domain: RollbackDomainId,
    pub tick: u32,
    pub sequence: u64,
}

impl RollbackEventId {
    pub const fn new(domain: RollbackDomainId, tick: u32, sequence: u64) -> Self {
        Self {
            domain,
            tick,
            sequence,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Reflect, Serialize, Deserialize)]
pub struct RollbackEvent<T> {
    pub id: RollbackEventId,
    pub source_command_tick: Option<u32>,
    pub entities: Vec<StableEntityId>,
    pub payload: T,
}

impl<T> Message for RollbackEvent<T> where T: Send + Sync + 'static {}

impl<T> RollbackEvent<T> {
    pub fn new(id: RollbackEventId, payload: T) -> Self {
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
pub struct RollbackEventDiff<T> {
    pub added: Vec<RollbackEvent<T>>,
    pub removed: Vec<RollbackEventId>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Reflect)]
pub struct RollbackCommit<T> {
    pub committed_tick: u32,
    pub added: Vec<RollbackEvent<T>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Reflect)]
pub struct RollbackEventStream<T> {
    provisional: Vec<RollbackEventBucket<T>>,
    committed: Vec<RollbackEventBucket<T>>,
    committed_tick: Option<u32>,
}

impl<T> Default for RollbackEventStream<T> {
    fn default() -> Self {
        Self {
            provisional: Vec::new(),
            committed: Vec::new(),
            committed_tick: None,
        }
    }
}

impl<T> RollbackEventStream<T> {
    pub fn provisional(&self) -> impl Iterator<Item = &RollbackEvent<T>> {
        self.provisional
            .iter()
            .flat_map(|bucket| bucket.events.iter())
    }

    pub fn committed(&self) -> impl Iterator<Item = &RollbackEvent<T>> {
        self.committed
            .iter()
            .flat_map(|bucket| bucket.events.iter())
    }

    pub fn provisional_event(&self, id: RollbackEventId) -> Option<&RollbackEvent<T>> {
        event_by_id(&self.provisional, id)
    }

    pub fn committed_event(&self, id: RollbackEventId) -> Option<&RollbackEvent<T>> {
        event_by_id(&self.committed, id)
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

impl<T> RollbackEventStream<T>
where
    T: Clone + PartialEq,
{
    /// Replaces the current replay-produced provisional stream.
    ///
    /// Visual systems consume the returned diff. Gameplay and durable business
    /// systems should not treat provisional removals as manual undo hooks; they
    /// should read the rebuilt provisional state or wait for committed events.
    pub fn replace_provisional(
        &mut self,
        events: impl IntoIterator<Item = RollbackEvent<T>>,
    ) -> RollbackEventDiff<T> {
        let committed_tick = self.committed_tick;
        let next_events = canonical_events(
            events
                .into_iter()
                .filter(|event| committed_tick.is_none_or(|tick| event.id.tick > tick))
                .collect(),
        );
        let RollbackEventDiff { added, removed } = diff_ordered(self.provisional(), &next_events);
        let next = buckets_from_sorted(next_events);

        self.provisional = next;
        RollbackEventDiff { added, removed }
    }

    /// Promotes provisional events at or before `committed_tick` into the final
    /// stream.
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
        let mut committed_events = self.committed().cloned().collect::<Vec<_>>();

        for bucket in ready {
            for event in bucket.events {
                let changed = self.committed_event(event.id) != Some(&event);
                if changed {
                    added.push(event.clone());
                }
                committed_events.push(event);
            }
        }

        self.committed = buckets_from_sorted(canonical_events(committed_events));
        self.committed_tick = Some(committed_tick);

        RollbackCommit {
            committed_tick,
            added,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Reflect)]
struct RollbackEventBucket<T> {
    tick: u32,
    events: Vec<RollbackEvent<T>>,
}

fn buckets_from_sorted<T>(events: Vec<RollbackEvent<T>>) -> Vec<RollbackEventBucket<T>> {
    let mut buckets = Vec::<RollbackEventBucket<T>>::new();
    for event in events {
        let needs_new_bucket = buckets
            .last()
            .is_none_or(|bucket| bucket.tick != event.id.tick);
        if needs_new_bucket {
            buckets.push(RollbackEventBucket {
                tick: event.id.tick,
                events: Vec::new(),
            });
        }

        let bucket_index = buckets.len() - 1;
        if buckets[bucket_index]
            .events
            .last()
            .is_some_and(|previous| previous.id == event.id)
        {
            *buckets[bucket_index].events.last_mut().unwrap() = event;
            continue;
        }
        buckets[bucket_index].events.push(event);
    }
    buckets
}

fn canonical_events<T>(events: Vec<RollbackEvent<T>>) -> Vec<RollbackEvent<T>> {
    let mut canonical = BTreeMap::new();
    for event in events {
        canonical.insert(event.id, event);
    }
    canonical.into_values().collect()
}

fn diff_ordered<'a, T>(
    previous: impl Iterator<Item = &'a RollbackEvent<T>>,
    next: &'a [RollbackEvent<T>],
) -> RollbackEventDiff<T>
where
    T: Clone + PartialEq + 'a,
{
    let mut previous = previous.peekable();
    let mut next_index = 0;
    let mut added = Vec::new();
    let mut removed = Vec::new();

    while let (Some(&previous_event), Some(next_event)) = (previous.peek(), next.get(next_index)) {
        match previous_event.id.cmp(&next_event.id) {
            std::cmp::Ordering::Less => {
                removed.push(previous_event.id);
                previous.next();
            }
            std::cmp::Ordering::Greater => {
                added.push(next_event.clone());
                next_index += 1;
            }
            std::cmp::Ordering::Equal => {
                if previous_event != next_event {
                    removed.push(previous_event.id);
                    added.push(next_event.clone());
                }
                previous.next();
                next_index += 1;
            }
        }
    }

    removed.extend(previous.map(|event| event.id));
    added.extend(next[next_index..].iter().cloned());

    RollbackEventDiff { added, removed }
}

fn event_by_id<T>(
    buckets: &[RollbackEventBucket<T>],
    id: RollbackEventId,
) -> Option<&RollbackEvent<T>> {
    let bucket = buckets
        .binary_search_by_key(&id.tick, |bucket| bucket.tick)
        .ok()
        .and_then(|index| buckets.get(index))?;
    bucket
        .events
        .binary_search_by_key(&id, |event| event.id)
        .ok()
        .and_then(|index| bucket.events.get(index))
}

#[cfg(test)]
mod tests;
