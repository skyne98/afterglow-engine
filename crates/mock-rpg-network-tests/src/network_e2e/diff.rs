use afterglow_engine::core::identity::StableEntityId;

use super::model::{CombatFact, CombatSnapshot, Correction};

pub(super) fn diff_snapshots(before: &CombatSnapshot, after: &CombatSnapshot) -> Vec<Correction> {
    let mut corrections = Vec::new();
    diff_component_set(
        &mut corrections,
        "Combatant",
        &before.combatants,
        &after.combatants,
    );
    diff_component_set(
        &mut corrections,
        "Inventory",
        &before.inventories,
        &after.inventories,
    );
    diff_component_set(
        &mut corrections,
        "Projectile",
        &before.projectiles,
        &after.projectiles,
    );
    diff_entity_set(
        &mut corrections,
        &before.death_markers,
        &after.death_markers,
    );
    diff_entity_set(&mut corrections, &before.corpses, &after.corpses);
    diff_component_set(&mut corrections, "Loot", &before.loot, &after.loot);
    diff_facts(&mut corrections, &before.log.facts, &after.log.facts);
    corrections
}

fn diff_component_set<T>(
    corrections: &mut Vec<Correction>,
    component: &'static str,
    before: &[(StableEntityId, T)],
    after: &[(StableEntityId, T)],
) where
    T: Eq,
{
    for (stable_id, old) in before {
        match after.iter().find(|(id, _)| id == stable_id) {
            Some((_, new)) if new != old => corrections.push(Correction::ComponentChanged {
                stable_id: *stable_id,
                component,
            }),
            None => corrections.push(Correction::DespawnEntity(*stable_id)),
            _ => {}
        }
    }
    for (stable_id, _) in after {
        if !before.iter().any(|(id, _)| id == stable_id) {
            corrections.push(Correction::SpawnEntity(*stable_id));
        }
    }
}

fn diff_entity_set<T>(
    corrections: &mut Vec<Correction>,
    before: &[(StableEntityId, T)],
    after: &[(StableEntityId, T)],
) {
    for (stable_id, _) in before {
        if !after.iter().any(|(id, _)| id == stable_id) {
            corrections.push(Correction::DespawnEntity(*stable_id));
        }
    }
    for (stable_id, _) in after {
        if !before.iter().any(|(id, _)| id == stable_id) {
            corrections.push(Correction::SpawnEntity(*stable_id));
        }
    }
}

fn diff_facts(corrections: &mut Vec<Correction>, before: &[CombatFact], after: &[CombatFact]) {
    for fact in before {
        if !after.contains(fact) {
            corrections.push(Correction::RemoveFact(fact.clone()));
        }
    }
    for fact in after {
        if !before.contains(fact) {
            corrections.push(Correction::AddFact(fact.clone()));
        }
    }
}
