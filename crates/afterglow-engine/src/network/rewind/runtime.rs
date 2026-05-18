use super::*;
use crate::core::identity::{RuntimeOnly, StableEntityId, StableIdAllocator};
use std::{
    any::TypeId,
    collections::{HashMap, HashSet},
    hash::Hasher,
};

type CaptureFn = fn(&World, Entity) -> Option<Vec<u8>>;

#[derive(Resource, Clone, Copy, Debug, Default, Eq, PartialEq, Reflect, Serialize, Deserialize)]
pub struct RewindTick(pub u32);

#[derive(Clone, Copy)]
pub struct RewindComponentRegistration {
    pub domain: RewindDomainId,
    pub type_key: u64,
    pub type_name: &'static str,
    capture: CaptureFn,
}

impl std::fmt::Debug for RewindComponentRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RewindComponentRegistration")
            .field("domain", &self.domain)
            .field("type_key", &self.type_key)
            .field("type_name", &self.type_name)
            .finish_non_exhaustive()
    }
}

#[derive(Resource, Default, Debug)]
pub struct RewindComponentRegistry {
    entries: Vec<RewindComponentRegistration>,
}

impl RewindComponentRegistry {
    pub fn register<T>(&mut self, domain: RewindDomainId)
    where
        T: Component + Serialize + TypePath + 'static,
    {
        let type_key = rewind_type_key::<T>();
        if self
            .entries
            .iter()
            .any(|entry| entry.domain == domain && entry.type_key == type_key)
        {
            return;
        }
        self.entries.push(RewindComponentRegistration {
            domain,
            type_key,
            type_name: T::type_path(),
            capture: capture_component::<T>,
        });
    }

    pub fn entries(&self) -> &[RewindComponentRegistration] {
        &self.entries
    }
}

#[derive(Resource, Default, Debug)]
pub struct RewindHistoryStore {
    histories: HashMap<(StableEntityId, u64), ComponentHistory>,
}

impl RewindHistoryStore {
    pub fn record_snapshot(
        &mut self,
        domain: RewindDomainId,
        stable_id: StableEntityId,
        type_key: u64,
        tick: u32,
        snapshot: Vec<u8>,
        budget: RewindHistoryBudget,
    ) {
        if budget.max_ticks == 0 {
            return;
        }
        let history = self
            .histories
            .entry((stable_id, type_key))
            .or_insert_with(|| ComponentHistory::with_capacity(budget.max_ticks, domain));
        if history.domain != domain {
            *history = ComponentHistory::with_capacity(budget.max_ticks, domain);
        }
        history.push(tick, snapshot, budget.drop_on_overflow);
    }

    pub fn history(&self, stable_id: StableEntityId, type_key: u64) -> Option<&ComponentHistory> {
        self.histories.get(&(stable_id, type_key))
    }

    pub fn history_mut(
        &mut self,
        stable_id: StableEntityId,
        type_key: u64,
    ) -> Option<&mut ComponentHistory> {
        self.histories.get_mut(&(stable_id, type_key))
    }

    pub fn prune_domain_up_to(&mut self, domain: RewindDomainId, tick: u32) {
        for history in self
            .histories
            .values_mut()
            .filter(|history| history.domain == domain)
        {
            history.prune_up_to(tick);
        }
        self.histories.retain(|_, history| !history.is_empty());
    }

    pub fn len(&self) -> usize {
        self.histories.len()
    }

    pub fn is_empty(&self) -> bool {
        self.histories.is_empty()
    }
}

#[derive(Resource, Default)]
pub(super) struct ServerRewindInstalled;

#[derive(Clone)]
struct CapturedSnapshot {
    domain: RewindDomainId,
    stable_id: StableEntityId,
    type_key: u64,
    tick: u32,
    snapshot: Vec<u8>,
    budget: RewindHistoryBudget,
}

pub fn rewind_type_key<T: 'static>() -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&TypeId::of::<T>(), &mut hasher);
    hasher.finish()
}

pub(super) fn ensure_rewinded_entities_have_stable_ids(world: &mut World) {
    if !world.contains_resource::<StableIdAllocator>() {
        world.insert_resource(StableIdAllocator::default());
    }

    let missing = {
        let mut query = world.query_filtered::<Entity, (
            With<RewindedEntity>,
            Without<RuntimeOnly>,
            Without<StableEntityId>,
        )>();
        query.iter(world).collect::<Vec<_>>()
    };
    let invalid = {
        let mut query = world.query_filtered::<(Entity, &StableEntityId), (
            With<RewindedEntity>,
            Without<RuntimeOnly>,
        )>();
        query
            .iter(world)
            .filter_map(|(entity, stable_id)| (!stable_id.is_valid()).then_some(entity))
            .collect::<Vec<_>>()
    };
    let mut reserved = {
        let mut query = world.query::<&StableEntityId>();
        query
            .iter(world)
            .copied()
            .filter(|id| id.is_valid())
            .collect::<HashSet<_>>()
    };

    for entity in missing.into_iter().chain(invalid) {
        let stable_id = world
            .resource_mut::<StableIdAllocator>()
            .allocate_excluding(&reserved);
        reserved.insert(stable_id);
        if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
            entity_mut.insert(stable_id);
        }
    }
}

pub(super) fn record_rewind_histories(world: &mut World) {
    let Some(registry) = world.get_resource::<RewindComponentRegistry>() else {
        return;
    };
    let Some(tick) = world.get_resource::<RewindTick>() else {
        return;
    };
    let Some(global_budget) = world.get_resource::<RewindHistoryBudget>() else {
        return;
    };
    let registrations = registry.entries().to_vec();
    let tick = tick.0;
    let global_budget = *global_budget;

    let mut query = world.query::<(Entity, &StableEntityId, &RewindedEntity)>();
    let entities = query
        .iter(world)
        .map(|(entity, stable_id, marker)| (entity, *stable_id, *marker))
        .collect::<Vec<_>>();

    let mut captured = Vec::new();
    for (entity, stable_id, marker) in entities {
        if !stable_id.is_valid() {
            continue;
        }
        let budget = RewindHistoryBudget {
            max_ticks: marker.budget_override.unwrap_or(global_budget.max_ticks),
            drop_on_overflow: global_budget.drop_on_overflow,
        };
        for registration in registrations
            .iter()
            .filter(|registration| registration.domain == marker.domain)
        {
            if let Some(snapshot) = (registration.capture)(world, entity) {
                captured.push(CapturedSnapshot {
                    domain: marker.domain,
                    stable_id,
                    type_key: registration.type_key,
                    tick,
                    snapshot,
                    budget,
                });
            }
        }
    }

    let mut store = world.resource_mut::<RewindHistoryStore>();
    for snapshot in captured {
        store.record_snapshot(
            snapshot.domain,
            snapshot.stable_id,
            snapshot.type_key,
            snapshot.tick,
            snapshot.snapshot,
            snapshot.budget,
        );
    }
}

fn capture_component<T>(world: &World, entity: Entity) -> Option<Vec<u8>>
where
    T: Component + Serialize,
{
    let component = world.get::<T>(entity)?;
    serde_json::to_vec(component).ok()
}
