use crate::core::identity::{Replicated, StableEntityId};
use bevy::{platform::collections::HashSet, prelude::*};
use std::{
    any::type_name,
    collections::{BTreeMap, BTreeSet},
    marker::PhantomData,
};

pub trait Replicate: Send + Sync + 'static {
    const REPLICATION_NAME: &'static str;
}

pub trait ReplicatedCommand: Message + Clone + PartialEq + Send + Sync + 'static {
    fn tick(&self) -> u32;
}

pub trait ReplicationEvent: Message + Clone + PartialEq + Send + Sync + 'static {
    fn tick(&self) -> u32;
}

#[derive(Resource, Clone, Debug, Default, Eq, PartialEq)]
pub struct ReplicationRegistry {
    pub components: HashSet<&'static str>,
    pub resources: HashSet<&'static str>,
    pub commands: HashSet<&'static str>,
    pub events: HashSet<&'static str>,
}

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct ReplicatedComponentState<T> {
    values: BTreeMap<StableEntityId, T>,
    removed: BTreeSet<StableEntityId>,
}

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct ReplicatedComponentEntityMap<T> {
    entities: BTreeMap<Entity, StableEntityId>,
    marker: PhantomData<T>,
}

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct ReplicatedResourceState<T> {
    value: Option<T>,
}

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct ReplicatedTimeline<T> {
    ticks: BTreeMap<u32, Vec<T>>,
    reissue: Vec<T>,
    reissued_pending_collection: BTreeMap<u32, Vec<T>>,
    retained_ticks: u32,
    latest_tick: Option<u32>,
}

impl<T> Default for ReplicatedComponentState<T> {
    fn default() -> Self {
        Self {
            values: BTreeMap::new(),
            removed: BTreeSet::new(),
        }
    }
}

impl<T> Default for ReplicatedResourceState<T> {
    fn default() -> Self {
        Self { value: None }
    }
}

impl<T> Default for ReplicatedComponentEntityMap<T> {
    fn default() -> Self {
        Self {
            entities: BTreeMap::new(),
            marker: PhantomData,
        }
    }
}

impl<T> Default for ReplicatedTimeline<T> {
    fn default() -> Self {
        Self {
            ticks: BTreeMap::new(),
            reissue: Vec::new(),
            reissued_pending_collection: BTreeMap::new(),
            retained_ticks: 120,
            latest_tick: None,
        }
    }
}

pub struct ReplicatedComponent<T>(PhantomData<T>);
pub struct ReplicatedResource<T>(PhantomData<T>);
pub struct ReplicatedCommandType<T>(PhantomData<T>);
pub struct ReplicatedEventType<T>(PhantomData<T>);

#[derive(Resource)]
struct RegisteredReplication<T, K>(PhantomData<fn() -> (T, K)>);

struct ComponentRegistration;
struct ResourceRegistration;
struct CommandRegistration;
struct EventRegistration;

fn register_once<T, K>(app: &mut App) -> bool
where
    RegisteredReplication<T, K>: Resource,
{
    let registered = app
        .world()
        .contains_resource::<RegisteredReplication<T, K>>();
    if registered {
        return false;
    }
    app.insert_resource(RegisteredReplication::<T, K>(PhantomData));
    true
}

pub fn component<T>() -> ReplicatedComponent<T> {
    ReplicatedComponent(PhantomData)
}

pub fn resource<T>() -> ReplicatedResource<T> {
    ReplicatedResource(PhantomData)
}

pub fn command<T>() -> ReplicatedCommandType<T> {
    ReplicatedCommandType(PhantomData)
}

pub fn event<T>() -> ReplicatedEventType<T> {
    ReplicatedEventType(PhantomData)
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, SystemSet)]
pub enum ReplicationSet {
    RestoreState,
    ReissueMessages,
    CollectEvents,
    CollectChanges,
}

pub trait ReplicationAppExt {
    fn replicate<R>(&mut self, registration: R) -> &mut Self
    where
        R: ReplicationRegistration;
}

pub trait ReplicationRegistration {
    fn register(self, app: &mut App);
}

impl<T> ReplicationRegistration for ReplicatedComponent<T>
where
    T: Component + Replicate + Clone,
{
    fn register(self, app: &mut App) {
        if !register_once::<T, ComponentRegistration>(app) {
            return;
        }
        app.init_resource::<ReplicationRegistry>()
            .init_resource::<ReplicatedComponentState<T>>()
            .init_resource::<ReplicatedComponentEntityMap<T>>()
            .add_systems(
                Update,
                (
                    collect_replicated_components::<T>,
                    collect_removed_replicated_components::<T>,
                    collect_unreplicated_components::<T>,
                )
                    .in_set(ReplicationSet::CollectChanges),
            );
        app.world_mut()
            .resource_mut::<ReplicationRegistry>()
            .components
            .insert(T::REPLICATION_NAME);
    }
}

impl<T> ReplicationRegistration for ReplicatedResource<T>
where
    T: Resource + Replicate + Clone,
{
    fn register(self, app: &mut App) {
        if !register_once::<T, ResourceRegistration>(app) {
            return;
        }
        app.init_resource::<ReplicationRegistry>()
            .init_resource::<ReplicatedResourceState<T>>()
            .add_systems(
                Update,
                collect_replicated_resource::<T>.in_set(ReplicationSet::CollectChanges),
            );
        app.world_mut()
            .resource_mut::<ReplicationRegistry>()
            .resources
            .insert(T::REPLICATION_NAME);
    }
}

impl<T> ReplicationRegistration for ReplicatedCommandType<T>
where
    T: ReplicatedCommand,
{
    fn register(self, app: &mut App) {
        if !register_once::<T, CommandRegistration>(app) {
            return;
        }
        app.add_message::<T>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<ReplicatedTimeline<T>>()
            .add_systems(
                Update,
                (
                    reissue_replicated_messages::<T>.in_set(ReplicationSet::ReissueMessages),
                    collect_replicated_commands::<T>.in_set(ReplicationSet::CollectChanges),
                ),
            );
        app.world_mut()
            .resource_mut::<ReplicationRegistry>()
            .commands
            .insert(type_name::<T>());
    }
}

impl<T> ReplicationRegistration for ReplicatedEventType<T>
where
    T: ReplicationEvent,
{
    fn register(self, app: &mut App) {
        if !register_once::<T, EventRegistration>(app) {
            return;
        }
        app.add_message::<T>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<ReplicatedTimeline<T>>()
            .add_systems(
                Update,
                (
                    reissue_replicated_messages::<T>.in_set(ReplicationSet::ReissueMessages),
                    collect_replicated_events::<T>.in_set(ReplicationSet::CollectChanges),
                ),
            );
        app.world_mut()
            .resource_mut::<ReplicationRegistry>()
            .events
            .insert(type_name::<T>());
    }
}

impl<T> ReplicatedComponentState<T> {
    pub fn get(&self, entity: StableEntityId) -> Option<&T> {
        self.values.get(&entity)
    }

    pub fn values(&self) -> &BTreeMap<StableEntityId, T> {
        &self.values
    }

    pub fn removed(&self) -> &BTreeSet<StableEntityId> {
        &self.removed
    }
}

impl<T> ReplicatedResourceState<T> {
    pub fn get(&self) -> Option<&T> {
        self.value.as_ref()
    }
}

impl<T> ReplicatedTimeline<T> {
    pub fn messages_at(&self, tick: u32) -> &[T] {
        self.ticks.get(&tick).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn replace_for_replay(&mut self, messages: impl IntoIterator<Item = (u32, T)>)
    where
        T: Clone,
    {
        self.ticks.clear();
        self.reissue.clear();
        self.reissued_pending_collection.clear();
        self.latest_tick = None;
        let messages = messages.into_iter().collect::<Vec<_>>();
        let oldest_retained_tick = messages
            .iter()
            .map(|(tick, _)| *tick)
            .max()
            .map(|tick| tick.saturating_sub(self.retained_ticks));
        for (tick, message) in messages {
            if oldest_retained_tick.is_some_and(|oldest| tick < oldest) {
                continue;
            }
            if !self.push_at(tick, message.clone()) {
                continue;
            }
            self.reissue.push(message.clone());
            self.reissued_pending_collection
                .entry(tick)
                .or_default()
                .push(message);
        }
    }

    pub fn push_at(&mut self, tick: u32, message: T) -> bool {
        let latest_tick = self.latest_tick.map_or(tick, |latest| latest.max(tick));
        self.latest_tick = Some(latest_tick);
        let oldest_retained_tick = latest_tick.saturating_sub(self.retained_ticks);
        if tick < oldest_retained_tick {
            return false;
        }
        self.ticks.entry(tick).or_default().push(message);
        self.prune_before(oldest_retained_tick);
        true
    }

    pub fn prune_before(&mut self, tick: u32) {
        while self
            .ticks
            .first_key_value()
            .is_some_and(|(oldest, _)| *oldest < tick)
        {
            let oldest = *self.ticks.first_key_value().unwrap().0;
            self.ticks.remove(&oldest);
            self.reissued_pending_collection.remove(&oldest);
        }
    }

    fn drain_reissue(&mut self) -> impl Iterator<Item = T> + '_ {
        self.reissue.drain(..)
    }

    fn consume_reissued_message(&mut self, tick: u32, message: &T) -> bool
    where
        T: PartialEq,
    {
        let Some(messages) = self.reissued_pending_collection.get_mut(&tick) else {
            return false;
        };
        let Some(index) = messages.iter().position(|pending| pending == message) else {
            return false;
        };
        messages.remove(index);
        if messages.is_empty() {
            self.reissued_pending_collection.remove(&tick);
        }
        true
    }
}

#[allow(clippy::type_complexity)]
fn collect_replicated_components<T>(
    query: Query<
        (Entity, &StableEntityId, &T),
        (
            With<Replicated>,
            Or<(Changed<T>, Changed<StableEntityId>, Added<Replicated>)>,
        ),
    >,
    mut state: ResMut<ReplicatedComponentState<T>>,
    mut entity_map: ResMut<ReplicatedComponentEntityMap<T>>,
) where
    T: Component + Replicate + Clone,
{
    let changes = query
        .iter()
        .map(|(entity, stable, value)| {
            (
                entity,
                entity_map.entities.get(&entity).copied(),
                *stable,
                value.clone(),
            )
        })
        .collect::<Vec<_>>();

    for (_, previous, stable, _) in &changes {
        if let Some(previous) = previous
            && previous != stable
        {
            state.values.remove(previous);
            state.removed.insert(*previous);
        }
    }

    for (entity, _, stable, value) in changes {
        state.values.insert(stable, value);
        state.removed.remove(&stable);
        entity_map.entities.insert(entity, stable);
    }
}

fn collect_unreplicated_components<T>(
    mut removed: RemovedComponents<Replicated>,
    current_replicated: Query<(), With<Replicated>>,
    mut state: ResMut<ReplicatedComponentState<T>>,
    mut entity_map: ResMut<ReplicatedComponentEntityMap<T>>,
) where
    T: Component + Replicate + Clone,
{
    for entity in removed.read() {
        if current_replicated.get(entity).is_ok() {
            continue;
        }
        let Some(stable) = entity_map.entities.remove(&entity) else {
            continue;
        };
        state.values.remove(&stable);
        state.removed.insert(stable);
    }
}

fn collect_removed_replicated_components<T>(
    mut removed: RemovedComponents<T>,
    current_components: Query<(), With<T>>,
    mut state: ResMut<ReplicatedComponentState<T>>,
    mut entity_map: ResMut<ReplicatedComponentEntityMap<T>>,
) where
    T: Component + Replicate + Clone,
{
    for entity in removed.read() {
        if current_components.get(entity).is_ok() {
            continue;
        }
        let Some(stable) = entity_map.entities.remove(&entity) else {
            continue;
        };
        state.values.remove(&stable);
        state.removed.insert(stable);
    }
}

fn collect_replicated_resource<T>(
    resource: Option<Res<T>>,
    mut state: ResMut<ReplicatedResourceState<T>>,
) where
    T: Resource + Replicate + Clone,
{
    let Some(resource) = resource else {
        if state.value.is_some() {
            state.value = None;
        }
        return;
    };
    if resource.is_changed() {
        state.value = Some(resource.clone());
    }
}

fn collect_replicated_commands<T>(
    mut messages: MessageReader<T>,
    mut timeline: ResMut<ReplicatedTimeline<T>>,
) where
    T: ReplicatedCommand,
{
    for message in messages.read().cloned() {
        if timeline.consume_reissued_message(message.tick(), &message) {
            continue;
        }
        timeline.push_at(message.tick(), message);
    }
}

fn collect_replicated_events<T>(
    mut messages: MessageReader<T>,
    mut timeline: ResMut<ReplicatedTimeline<T>>,
) where
    T: ReplicationEvent,
{
    for message in messages.read().cloned() {
        if timeline.consume_reissued_message(message.tick(), &message) {
            continue;
        }
        timeline.push_at(message.tick(), message);
    }
}

fn reissue_replicated_messages<T>(
    mut timeline: ResMut<ReplicatedTimeline<T>>,
    mut messages: ResMut<Messages<T>>,
) where
    T: Message + Clone,
{
    for message in timeline.drain_reissue() {
        messages.write(message);
    }
}

pub fn configure_replication_sets(app: &mut App) {
    app.configure_sets(
        Update,
        (
            ReplicationSet::RestoreState,
            ReplicationSet::ReissueMessages,
            ReplicationSet::CollectEvents,
            ReplicationSet::CollectChanges,
        )
            .chain()
            .in_set(crate::core::schedule::AfterglowSet::ApplyGameplay),
    );
}
