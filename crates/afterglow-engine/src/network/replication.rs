use crate::core::identity::StableEntityId;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReplicationWorld {
    entities: BTreeMap<StableEntityId, ReplicatedEntityState>,
    dirty_entities: BTreeSet<StableEntityId>,
    removed_entities: BTreeSet<StableEntityId>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReplicatedEntityState {
    fields: BTreeMap<String, Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldSnapshot {
    pub tick: u32,
    pub entities: Vec<EntitySnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntitySnapshot {
    pub entity: StableEntityId,
    pub fields: Vec<FieldValue>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldDelta {
    pub from_tick: u32,
    pub to_tick: u32,
    pub changes: Vec<EntityDelta>,
    pub removed: Vec<StableEntityId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntityDelta {
    pub entity: StableEntityId,
    pub changed: Vec<FieldValue>,
    pub removed: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldValue {
    pub name: String,
    pub value: Vec<u8>,
}

impl ReplicationWorld {
    pub fn set_field(
        &mut self,
        entity: StableEntityId,
        name: impl Into<String>,
        value: impl Into<Vec<u8>>,
    ) {
        let name = name.into();
        let value = value.into();
        let state = self.entities.entry(entity).or_default();
        if state.fields.get(&name) != Some(&value) {
            state.fields.insert(name, value);
            self.dirty_entities.insert(entity);
            self.removed_entities.remove(&entity);
        }
    }

    pub fn remove_field(&mut self, entity: StableEntityId, name: &str) {
        if let Some(state) = self.entities.get_mut(&entity)
            && state.fields.remove(name).is_some()
        {
            self.dirty_entities.insert(entity);
        }
    }

    pub fn remove_entity(&mut self, entity: StableEntityId) {
        if self.entities.remove(&entity).is_some() {
            self.dirty_entities.remove(&entity);
            self.removed_entities.insert(entity);
        }
    }

    pub fn entity(&self, entity: StableEntityId) -> Option<&ReplicatedEntityState> {
        self.entities.get(&entity)
    }

    pub fn snapshot(&self, tick: u32) -> WorldSnapshot {
        WorldSnapshot {
            tick,
            entities: self
                .entities
                .iter()
                .map(|(entity, state)| EntitySnapshot {
                    entity: *entity,
                    fields: state.field_values(),
                })
                .collect(),
        }
    }

    pub fn delta_since(&self, baseline: &WorldSnapshot, tick: u32) -> WorldDelta {
        let mut changes = Vec::new();
        let mut removed = Vec::new();
        let mut baseline_index = 0;

        for (entity, current) in &self.entities {
            while let Some(baseline_entity) = baseline.entities.get(baseline_index) {
                if baseline_entity.entity >= *entity {
                    break;
                }
                removed.push(baseline_entity.entity);
                baseline_index += 1;
            }

            let baseline_state = baseline
                .entities
                .get(baseline_index)
                .filter(|state| state.entity == *entity);
            let changed = current.changed_fields(baseline_state);
            let removed_fields = baseline_state
                .map(|state| state.removed_fields(current))
                .unwrap_or_default();

            if !changed.is_empty() || !removed_fields.is_empty() {
                changes.push(EntityDelta {
                    entity: *entity,
                    changed,
                    removed: removed_fields,
                });
            }

            if baseline_state.is_some() {
                baseline_index += 1;
            }
        }
        removed.extend(
            baseline.entities[baseline_index..]
                .iter()
                .map(|entity| entity.entity),
        );

        WorldDelta {
            from_tick: baseline.tick,
            to_tick: tick,
            changes,
            removed,
        }
    }

    pub fn dirty_delta_since(&self, baseline: &WorldSnapshot, tick: u32) -> WorldDelta {
        let mut changes = Vec::new();
        for entity in &self.dirty_entities {
            let Some(current) = self.entities.get(entity) else {
                continue;
            };
            let baseline_state = baseline.entity(*entity);
            let changed = current.changed_fields(baseline_state);
            let removed_fields = baseline_state
                .map(|state| state.removed_fields(current))
                .unwrap_or_default();
            if !changed.is_empty() || !removed_fields.is_empty() {
                changes.push(EntityDelta {
                    entity: *entity,
                    changed,
                    removed: removed_fields,
                });
            }
        }

        WorldDelta {
            from_tick: baseline.tick,
            to_tick: tick,
            changes,
            removed: self.removed_entities.iter().copied().collect(),
        }
    }

    pub fn apply_snapshot(&mut self, snapshot: &WorldSnapshot) {
        *self = Self::from_snapshot(snapshot);
    }

    pub fn apply_delta(&mut self, delta: &WorldDelta) {
        for entity in &delta.removed {
            self.entities.remove(entity);
        }
        for entity_delta in &delta.changes {
            let state = self.entities.entry(entity_delta.entity).or_default();
            for field in &entity_delta.removed {
                state.fields.remove(field);
            }
            for field in &entity_delta.changed {
                state.fields.insert(field.name.clone(), field.value.clone());
            }
        }
        self.clear_changes();
    }

    pub fn clear_changes(&mut self) {
        self.dirty_entities.clear();
        self.removed_entities.clear();
    }

    fn from_snapshot(snapshot: &WorldSnapshot) -> Self {
        let entities = snapshot
            .entities
            .iter()
            .map(|entity| {
                (
                    entity.entity,
                    ReplicatedEntityState {
                        fields: entity
                            .fields
                            .iter()
                            .map(|field| (field.name.clone(), field.value.clone()))
                            .collect(),
                    },
                )
            })
            .collect();
        Self {
            entities,
            dirty_entities: BTreeSet::new(),
            removed_entities: BTreeSet::new(),
        }
    }
}

impl ReplicatedEntityState {
    pub fn field(&self, name: &str) -> Option<&[u8]> {
        self.fields.get(name).map(Vec::as_slice)
    }

    fn field_values(&self) -> Vec<FieldValue> {
        self.fields
            .iter()
            .map(|(name, value)| FieldValue {
                name: name.clone(),
                value: value.clone(),
            })
            .collect()
    }

    fn changed_fields(&self, baseline: Option<&EntitySnapshot>) -> Vec<FieldValue> {
        self.fields
            .iter()
            .filter(|(name, value)| {
                baseline.and_then(|state| state.field(name)) != Some(value.as_slice())
            })
            .map(|(name, value)| FieldValue {
                name: name.clone(),
                value: value.clone(),
            })
            .collect()
    }
}

impl EntitySnapshot {
    fn entity(&self) -> StableEntityId {
        self.entity
    }
}

impl WorldSnapshot {
    fn entity(&self, entity: StableEntityId) -> Option<&EntitySnapshot> {
        self.entities
            .binary_search_by_key(&entity, EntitySnapshot::entity)
            .ok()
            .map(|index| &self.entities[index])
    }
}

impl EntitySnapshot {
    fn field(&self, name: &str) -> Option<&[u8]> {
        self.fields
            .binary_search_by_key(&name, |field| field.name.as_str())
            .ok()
            .map(|index| self.fields[index].value.as_slice())
    }

    fn removed_fields(&self, current: &ReplicatedEntityState) -> Vec<String> {
        self.fields
            .iter()
            .filter(|field| !current.fields.contains_key(&field.name))
            .map(|field| field.name.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests;
