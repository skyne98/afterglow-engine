use super::{
    Replicate, ReplicatedComponentHistory, ReplicatedResourceHistory, ReplicatedTimeline,
    history::{
        restore_component_history, restore_resource_history, save_component_history,
        save_resource_history,
    },
};
use crate::core::identity::{Replicated, maintain_stable_entity_registry};
use bevy::prelude::*;

#[derive(Resource, Default)]
pub(crate) struct ReplicationRuntimeRegistry {
    has_state: Vec<fn(&World, u32) -> bool>,
    save_state: Vec<fn(&mut World, u32)>,
    restore_state: Vec<fn(&mut World, u32) -> bool>,
    reissue_messages: Vec<fn(&mut World, u32)>,
    entity_has_state: Vec<fn(&World, Entity) -> bool>,
}

impl ReplicationRuntimeRegistry {
    fn has_state(&self) -> Vec<fn(&World, u32) -> bool> {
        self.has_state.clone()
    }

    pub(crate) fn save_state(&self) -> Vec<fn(&mut World, u32)> {
        self.save_state.clone()
    }

    pub(crate) fn restore_state(&self) -> Vec<fn(&mut World, u32) -> bool> {
        self.restore_state.clone()
    }

    pub(crate) fn reissue_messages(&self) -> Vec<fn(&mut World, u32)> {
        self.reissue_messages.clone()
    }

    fn entity_has_state(&self) -> Vec<fn(&World, Entity) -> bool> {
        self.entity_has_state.clone()
    }
}

pub(crate) fn register_component_runtime<T>(app: &mut App)
where
    T: Component + Replicate + Clone,
{
    app.init_resource::<ReplicatedComponentHistory<T>>()
        .init_resource::<ReplicationRuntimeRegistry>();
    let mut registry = app.world_mut().resource_mut::<ReplicationRuntimeRegistry>();
    registry.has_state.push(component_history_has_tick::<T>);
    registry.save_state.push(save_component_history::<T>);
    registry.restore_state.push(restore_component_history::<T>);
    registry.entity_has_state.push(entity_has_component::<T>);
}

pub(crate) fn register_resource_runtime<T>(app: &mut App)
where
    T: Resource + Replicate + Clone,
{
    app.init_resource::<ReplicatedResourceHistory<T>>()
        .init_resource::<ReplicationRuntimeRegistry>();
    let mut registry = app.world_mut().resource_mut::<ReplicationRuntimeRegistry>();
    registry.has_state.push(resource_history_has_tick::<T>);
    registry.save_state.push(save_resource_history::<T>);
    registry.restore_state.push(restore_resource_history::<T>);
}

pub(crate) fn register_timeline_runtime<T>(app: &mut App)
where
    T: Message + Clone + PartialEq,
{
    app.init_resource::<ReplicationRuntimeRegistry>();
    app.world_mut()
        .resource_mut::<ReplicationRuntimeRegistry>()
        .reissue_messages
        .push(reissue_timeline_messages::<T>);
}

pub(crate) fn run_save_callbacks(world: &mut World, tick: u32) {
    let callbacks = world.resource::<ReplicationRuntimeRegistry>().save_state();
    for callback in callbacks {
        callback(world, tick);
    }
}

pub(crate) fn run_restore_callbacks(world: &mut World, tick: u32) -> bool {
    let preflight = world.resource::<ReplicationRuntimeRegistry>().has_state();
    if preflight
        .into_iter()
        .any(|snapshot_exists| !snapshot_exists(world, tick))
    {
        return false;
    }

    maintain_stable_entity_registry(world);
    let callbacks = world
        .resource::<ReplicationRuntimeRegistry>()
        .restore_state();
    let mut restored = true;
    for callback in callbacks {
        restored &= callback(world, tick);
    }
    if restored {
        despawn_empty_replicated_entities(world);
        maintain_stable_entity_registry(world);
    }
    restored
}

pub(crate) fn run_reissue_callbacks(world: &mut World, tick: u32) {
    let callbacks = world
        .resource::<ReplicationRuntimeRegistry>()
        .reissue_messages();
    for callback in callbacks {
        callback(world, tick);
    }
}

fn reissue_timeline_messages<T>(world: &mut World, tick: u32)
where
    T: Message + Clone + PartialEq,
{
    let messages = {
        let mut timeline = world.resource_mut::<ReplicatedTimeline<T>>();
        let messages = timeline.messages_at(tick).to_vec();
        if messages.is_empty() {
            return;
        }
        let mut newly_pending = Vec::new();
        for message in &messages {
            if let Some(index) = timeline
                .reissue
                .iter()
                .position(|pending| pending == message)
            {
                timeline.reissue.remove(index);
            } else {
                newly_pending.push(message.clone());
            }
        }
        if !newly_pending.is_empty() {
            timeline
                .reissued_pending_collection
                .entry(tick)
                .or_default()
                .extend(newly_pending);
        }
        messages
    };

    let mut bevy_messages = world.resource_mut::<Messages<T>>();
    for message in messages {
        bevy_messages.write(message);
    }
}

fn component_history_has_tick<T>(world: &World, tick: u32) -> bool
where
    T: Component,
{
    world
        .resource::<ReplicatedComponentHistory<T>>()
        .snapshot(tick)
        .is_some()
}

fn resource_history_has_tick<T>(world: &World, tick: u32) -> bool
where
    T: Resource,
{
    world
        .resource::<ReplicatedResourceHistory<T>>()
        .snapshot(tick)
        .is_some()
}

fn entity_has_component<T>(world: &World, entity: Entity) -> bool
where
    T: Component,
{
    world.get::<T>(entity).is_some()
}

fn despawn_empty_replicated_entities(world: &mut World) {
    let callbacks = world
        .resource::<ReplicationRuntimeRegistry>()
        .entity_has_state();
    if callbacks.is_empty() {
        return;
    }

    let entities = {
        let mut query = world.query_filtered::<Entity, With<Replicated>>();
        query.iter(world).collect::<Vec<_>>()
    };
    for entity in entities {
        if callbacks.iter().any(|callback| callback(world, entity)) {
            continue;
        }
        if let Ok(entity_mut) = world.get_entity_mut(entity) {
            entity_mut.despawn();
        }
    }
}
