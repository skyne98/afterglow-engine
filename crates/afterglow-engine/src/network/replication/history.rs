use super::{
    Replicate, ReplicatedComponentEntityMap, ReplicatedComponentState, ReplicatedResourceState,
    RollbackReplicationClock,
};
use crate::core::identity::{Replicated, StableEntityId, StableEntityRegistry};
use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplicatedComponentSnapshot<T> {
    values: BTreeMap<StableEntityId, T>,
}

#[derive(Resource, Clone, Debug, Eq, PartialEq)]
pub struct ReplicatedComponentHistory<T> {
    snapshots: BTreeMap<u32, ReplicatedComponentSnapshot<T>>,
    retained_ticks: u32,
}

#[derive(Resource, Clone, Debug, Eq, PartialEq)]
pub struct ReplicatedResourceHistory<T> {
    snapshots: BTreeMap<u32, Option<T>>,
    retained_ticks: u32,
}

impl<T> Default for ReplicatedComponentHistory<T> {
    fn default() -> Self {
        Self {
            snapshots: BTreeMap::new(),
            retained_ticks: 120,
        }
    }
}

impl<T> Default for ReplicatedResourceHistory<T> {
    fn default() -> Self {
        Self {
            snapshots: BTreeMap::new(),
            retained_ticks: 120,
        }
    }
}

impl<T> ReplicatedComponentSnapshot<T> {
    pub fn values(&self) -> &BTreeMap<StableEntityId, T> {
        &self.values
    }
}

impl<T> ReplicatedComponentHistory<T> {
    pub fn snapshot(&self, tick: u32) -> Option<&ReplicatedComponentSnapshot<T>> {
        self.snapshots.get(&tick)
    }

    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }
}

impl<T> ReplicatedResourceHistory<T> {
    pub fn snapshot(&self, tick: u32) -> Option<&Option<T>> {
        self.snapshots.get(&tick)
    }

    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }
}

pub(crate) fn save_component_history<T>(world: &mut World, tick: u32)
where
    T: Component + Replicate + Clone,
{
    let retained_ticks = retained_ticks(world);
    let canonical_entities = canonical_entities_by_stable_id(world);
    let mut query = world.query_filtered::<(Entity, &StableEntityId, &T), With<Replicated>>();
    let mut selected = BTreeMap::<StableEntityId, (u8, Entity, T)>::new();

    for (entity, stable, value) in query.iter(world) {
        if !stable.is_valid() {
            continue;
        }
        let rank = u8::from(
            canonical_entities
                .get(stable)
                .is_none_or(|canonical| *canonical != entity),
        );
        match selected.entry(*stable) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert((rank, entity, value.clone()));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let (current_rank, current_entity, _) = entry.get();
                if (rank, entity) < (*current_rank, *current_entity) {
                    entry.insert((rank, entity, value.clone()));
                }
            }
        }
    }

    let values = selected
        .iter()
        .map(|(stable, (_, _, value))| (*stable, value.clone()))
        .collect::<BTreeMap<_, _>>();
    let entities = selected
        .iter()
        .map(|(stable, (_, entity, _))| (*entity, *stable))
        .collect::<BTreeMap<_, _>>();

    let mut history = world.resource_mut::<ReplicatedComponentHistory<T>>();
    history.retained_ticks = retained_ticks;
    history.snapshots.insert(
        tick,
        ReplicatedComponentSnapshot {
            values: values.clone(),
        },
    );
    prune_before(&mut history.snapshots, tick.saturating_sub(retained_ticks));

    world.resource_mut::<ReplicatedComponentState<T>>().values = values;
    world
        .resource_mut::<ReplicatedComponentState<T>>()
        .removed
        .clear();
    world
        .resource_mut::<ReplicatedComponentEntityMap<T>>()
        .entities = entities;
}

pub(crate) fn restore_component_history<T>(world: &mut World, tick: u32) -> bool
where
    T: Component + Replicate + Clone,
{
    let Some(snapshot) = world
        .resource::<ReplicatedComponentHistory<T>>()
        .snapshot(tick)
        .cloned()
    else {
        return false;
    };
    let canonical_entities = canonical_entities_by_stable_id(world);
    let current = {
        let mut query = world
            .query_filtered::<(Entity, Option<&StableEntityId>), (With<Replicated>, With<T>)>();
        query
            .iter(world)
            .map(|(entity, stable)| (entity, stable.copied()))
            .collect::<Vec<_>>()
    };

    for (entity, stable) in current {
        let Some(stable) = stable.filter(|stable| stable.is_valid()) else {
            if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
                entity_mut.remove::<T>();
            }
            continue;
        };
        let in_snapshot = snapshot.values.contains_key(&stable);
        let duplicate = in_snapshot
            && canonical_entities
                .get(&stable)
                .is_some_and(|canonical| *canonical != entity);
        if (!in_snapshot || duplicate)
            && let Ok(mut entity_mut) = world.get_entity_mut(entity)
        {
            entity_mut.remove::<T>();
        }
    }

    for (stable, value) in snapshot.values.clone() {
        let entity = canonical_entities
            .get(&stable)
            .copied()
            .unwrap_or_else(|| world.spawn((stable, Replicated)).id());
        world.entity_mut(entity).insert((Replicated, value));
    }

    save_restored_component_state::<T>(world, snapshot.values);
    true
}

pub(crate) fn save_resource_history<T>(world: &mut World, tick: u32)
where
    T: Resource + Replicate + Clone,
{
    let retained_ticks = retained_ticks(world);
    let value = world.get_resource::<T>().cloned();
    let mut history = world.resource_mut::<ReplicatedResourceHistory<T>>();
    history.retained_ticks = retained_ticks;
    history.snapshots.insert(tick, value.clone());
    prune_before(&mut history.snapshots, tick.saturating_sub(retained_ticks));
    world.resource_mut::<ReplicatedResourceState<T>>().value = value;
}

pub(crate) fn restore_resource_history<T>(world: &mut World, tick: u32) -> bool
where
    T: Resource + Replicate + Clone,
{
    let Some(value) = world
        .resource::<ReplicatedResourceHistory<T>>()
        .snapshot(tick)
        .cloned()
    else {
        return false;
    };

    if let Some(value) = value.clone() {
        world.insert_resource(value);
    } else {
        world.remove_resource::<T>();
    }
    world.resource_mut::<ReplicatedResourceState<T>>().value = value;
    true
}

fn save_restored_component_state<T>(world: &mut World, values: BTreeMap<StableEntityId, T>)
where
    T: Component + Replicate + Clone,
{
    let entities = {
        let mut query =
            world.query_filtered::<(Entity, &StableEntityId), (With<Replicated>, With<T>)>();
        query
            .iter(world)
            .filter(|(_, stable)| stable.is_valid())
            .map(|(entity, stable)| (entity, *stable))
            .collect::<BTreeMap<_, _>>()
    };
    let mut state = world.resource_mut::<ReplicatedComponentState<T>>();
    state.values = values;
    state.removed.clear();
    world
        .resource_mut::<ReplicatedComponentEntityMap<T>>()
        .entities = entities;
}

fn canonical_entities_by_stable_id(world: &mut World) -> BTreeMap<StableEntityId, Entity> {
    let mut query = world.query::<(Entity, &StableEntityId, Option<&Replicated>)>();
    let stable_ids = query
        .iter(world)
        .filter_map(|(_, stable, _)| stable.is_valid().then_some(*stable))
        .collect::<BTreeSet<_>>();

    if let Some(registry) = world.get_resource::<StableEntityRegistry>() {
        return stable_ids
            .into_iter()
            .filter_map(|stable| registry.entity(stable).map(|entity| (stable, entity)))
            .collect();
    }

    let mut canonical = BTreeMap::<StableEntityId, (u8, Entity)>::new();

    for (entity, stable, replicated) in query.iter(world) {
        if !stable.is_valid() {
            continue;
        }
        let rank = u8::from(replicated.is_none());
        match canonical.entry(*stable) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert((rank, entity));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                if (rank, entity) < *entry.get() {
                    entry.insert((rank, entity));
                }
            }
        }
    }

    canonical
        .into_iter()
        .map(|(stable, (_, entity))| (stable, entity))
        .collect()
}

fn retained_ticks(world: &World) -> u32 {
    world
        .get_resource::<RollbackReplicationClock>()
        .map(|clock| clock.policy.max_rollback_ticks)
        .unwrap_or(120)
}

fn prune_before<T>(snapshots: &mut BTreeMap<u32, T>, tick: u32) {
    while snapshots
        .first_key_value()
        .is_some_and(|(oldest, _)| *oldest < tick)
    {
        let oldest = *snapshots.first_key_value().unwrap().0;
        snapshots.remove(&oldest);
    }
}
