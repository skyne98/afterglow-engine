use afterglow_engine::core::identity::StableEntityId;
use bevy::prelude::*;

use super::model::{
    CombatLog, CombatSnapshot, Combatant, Corpse, DeathMarker, Inventory, Loot, Projectile,
};

pub(super) fn capture_snapshot(app: &mut App) -> CombatSnapshot {
    CombatSnapshot {
        combatants: sorted_components::<Combatant>(app),
        inventories: sorted_components::<Inventory>(app),
        projectiles: sorted_components::<Projectile>(app),
        death_markers: sorted_components::<DeathMarker>(app),
        corpses: sorted_components::<Corpse>(app),
        loot: sorted_components::<Loot>(app),
        log: app.world().resource::<CombatLog>().clone(),
    }
}

pub(super) fn restore_snapshot(app: &mut App, snapshot: CombatSnapshot) {
    let CombatSnapshot {
        combatants,
        inventories,
        projectiles,
        death_markers,
        corpses,
        loot,
        log,
    } = snapshot;
    despawn_all::<Combatant>(app.world_mut());
    despawn_all::<Inventory>(app.world_mut());
    despawn_all::<Projectile>(app.world_mut());
    despawn_all::<DeathMarker>(app.world_mut());
    despawn_all::<Corpse>(app.world_mut());
    despawn_all::<Loot>(app.world_mut());
    app.world_mut().insert_resource(log);
    for (stable, combatant) in combatants {
        let inventory = inventories
            .iter()
            .find_map(|(id, inventory)| (*id == stable).then_some(inventory.clone()))
            .unwrap_or_default();
        app.world_mut().spawn((stable, combatant, inventory));
    }
    for (stable, projectile) in projectiles {
        app.world_mut().spawn((stable, projectile));
    }
    for (stable, marker) in death_markers {
        app.world_mut().spawn((stable, marker));
    }
    for (stable, corpse) in corpses {
        app.world_mut().spawn((stable, corpse));
    }
    for (stable, loot) in loot {
        app.world_mut().spawn((stable, loot));
    }
}

pub(super) fn sorted_components<T>(app: &mut App) -> Vec<(StableEntityId, T)>
where
    T: Component + Clone,
{
    let mut query = app.world_mut().query::<(&StableEntityId, &T)>();
    let mut values = query
        .iter(app.world())
        .map(|(stable, component)| (*stable, component.clone()))
        .collect::<Vec<_>>();
    values.sort_by_key(|(stable, _)| *stable);
    values
}

pub(super) fn despawn_all<T: Component>(world: &mut World) {
    let mut query = world.query_filtered::<Entity, With<T>>();
    let entities = query.iter(world).collect::<Vec<_>>();
    for entity in entities {
        world.entity_mut(entity).despawn();
    }
}

pub(super) fn projectile_id(
    tick: u32,
    caster: StableEntityId,
    target: StableEntityId,
) -> StableEntityId {
    StableEntityId::from_raw(
        10_000 + u128::from(tick) * 1_000 + caster.as_raw() * 10 + target.as_raw(),
    )
}

pub(super) fn death_marker_id(victim: StableEntityId) -> StableEntityId {
    StableEntityId::from_raw(20_000 + victim.as_raw())
}

pub(super) fn corpse_id(victim: StableEntityId) -> StableEntityId {
    StableEntityId::from_raw(25_000 + victim.as_raw())
}

pub(super) fn loot_id(victim: StableEntityId) -> StableEntityId {
    StableEntityId::from_raw(30_000 + victim.as_raw())
}

pub(super) fn retained_snapshot_floor(current_tick: u32, retention_ticks: u32) -> u32 {
    current_tick
        .saturating_sub(retention_ticks)
        .saturating_sub(1)
}
