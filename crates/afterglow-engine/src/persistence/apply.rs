use super::*;

struct EntityApplyPlan {
    entity: StableEntityId,
    apply_components: Vec<Box<dyn PersistentComponentApply>>,
    remove_components: Vec<String>,
}

pub fn apply_chunk_delta(
    world: &mut World,
    delta: &ChunkPersistentDelta,
) -> Result<ChunkDeltaApplyReport, PersistenceError> {
    apply_chunk_deltas(world, [delta])
}

pub fn apply_chunk_deltas<'a>(
    world: &mut World,
    deltas: impl IntoIterator<Item = &'a ChunkPersistentDelta>,
) -> Result<ChunkDeltaApplyReport, PersistenceError> {
    let deltas = deltas.into_iter().collect::<Vec<_>>();
    validate_apply_deltas(&deltas)?;
    maintain_stable_entity_registry(world);
    let plans = deltas
        .iter()
        .copied()
        .map(|delta| build_apply_plan(world, delta).map(|plan| (delta, plan)))
        .collect::<Result<Vec<_>, _>>()?;
    let mut report = ChunkDeltaApplyReport::default();

    for (delta, _) in &plans {
        for stable_id in &delta.deleted {
            if let Some(entity) = entity_in_chunk(world, *stable_id, delta.chunk)
                && world.despawn(entity)
            {
                report.despawned += 1;
            }
        }
    }
    maintain_stable_entity_registry(world);

    for (delta, plan) in plans {
        for entity_plan in plan {
            if delta.deleted.contains(&entity_plan.entity) {
                continue;
            }
            let (entity, spawned) = get_or_spawn_entity(world, delta.chunk, entity_plan.entity);
            if spawned {
                report.spawned += 1;
            } else {
                report.updated += 1;
            }

            for type_name in &entity_plan.remove_components {
                let remove = {
                    let registry = world.resource::<PersistenceRegistry>();
                    registry
                        .components
                        .get(type_name.as_str())
                        .map(|runtime| runtime.remove)
                };
                if remove.is_some_and(|remove| remove(world, entity)) {
                    report.components_removed += 1;
                }
            }
            for component in entity_plan.apply_components {
                component.apply(world, entity);
                report.components_applied += 1;
            }
        }
    }
    maintain_stable_entity_registry(world);
    Ok(report)
}

fn validate_apply_deltas(deltas: &[&ChunkPersistentDelta]) -> Result<(), PersistenceError> {
    let mut restored = BTreeSet::new();
    let mut deleted = BTreeSet::new();

    for delta in deltas {
        if !delta.chunk.is_valid() {
            return Err(PersistenceError::InvalidChunkId);
        }
        for entity_delta in &delta.entities {
            if !entity_delta.entity.is_valid() {
                return Err(PersistenceError::InvalidEntityId);
            }
            if !restored.insert(entity_delta.entity) {
                return Err(PersistenceError::DuplicateEntityDelta {
                    entity: entity_delta.entity,
                });
            }
            if deleted.contains(&entity_delta.entity) {
                return Err(PersistenceError::ConflictingEntityDelta {
                    entity: entity_delta.entity,
                });
            }
        }
        for entity in &delta.deleted {
            if !entity.is_valid() {
                return Err(PersistenceError::InvalidEntityId);
            }
            if restored.contains(entity) || !deleted.insert(*entity) {
                return Err(PersistenceError::ConflictingEntityDelta { entity: *entity });
            }
        }
    }
    Ok(())
}

fn build_apply_plan(
    world: &World,
    delta: &ChunkPersistentDelta,
) -> Result<Vec<EntityApplyPlan>, PersistenceError> {
    let registry = world.resource::<PersistenceRegistry>();
    delta
        .entities
        .iter()
        .map(|entity_delta| {
            let present = entity_delta
                .components
                .iter()
                .map(|component| component.type_name.as_str())
                .collect::<BTreeSet<_>>();
            let mut apply_components = Vec::new();
            for component in &entity_delta.components {
                let (type_name, runtime) = registry
                    .components
                    .get_key_value(component.type_name.as_str())
                    .ok_or_else(|| PersistenceError::UnregisteredComponent {
                        type_name: component.type_name.clone(),
                    })?;
                apply_components.push((runtime.deserialize)(&component.payload, type_name)?);
            }
            let remove_components = entity_delta
                .removed_components
                .iter()
                .filter(|type_name| !present.contains(type_name.as_str()))
                .map(|type_name| {
                    if registry.components.contains_key(type_name.as_str()) {
                        Ok(type_name.clone())
                    } else {
                        Err(PersistenceError::UnregisteredComponent {
                            type_name: type_name.clone(),
                        })
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(EntityApplyPlan {
                entity: entity_delta.entity,
                apply_components,
                remove_components,
            })
        })
        .collect()
}

fn entity_in_chunk(world: &World, stable_id: StableEntityId, chunk: ChunkId) -> Option<Entity> {
    let entity = world.resource::<StableEntityRegistry>().entity(stable_id)?;
    (world
        .get::<ChunkMembership>(entity)
        .map(|membership| membership.chunk)
        == Some(chunk))
    .then_some(entity)
}

fn get_or_spawn_entity(
    world: &mut World,
    chunk: ChunkId,
    stable_id: StableEntityId,
) -> (Entity, bool) {
    if let Some(entity) = world.resource::<StableEntityRegistry>().entity(stable_id) {
        world
            .entity_mut(entity)
            .insert((Persistent, ChunkMembership::new(chunk)));
        return (entity, false);
    }
    let entity = world
        .spawn((Persistent, stable_id, ChunkMembership::new(chunk)))
        .id();
    (entity, true)
}
