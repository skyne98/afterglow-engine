use afterglow_engine::{
    core::{AfterglowCorePlugin, identity::StableEntityId},
    network::{AfterglowLightyearConfig, AfterglowNetworkPlugin, HistoryTick},
};
use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

use super::{diff::diff_snapshots, model::*, net::MockNetwork, world::*};
use crate::{Vec3i, in_reach, valid_move};

pub struct NetworkedRpg {
    app: App,
    network: MockNetwork,
    accepted_inputs: Vec<ClientInput>,
    seen_sequences: BTreeSet<(StableEntityId, u64)>,
    snapshots: BTreeMap<u32, CombatSnapshot>,
    current_tick: u32,
    retention_ticks: u32,
    corrections: Vec<Correction>,
    rejected: Vec<RejectedInput>,
}

impl NetworkedRpg {
    pub fn new(retention_ticks: u32) -> Self {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AfterglowCorePlugin, AfterglowNetworkPlugin));
        app.insert_resource(CombatLog::default());
        app.init_resource::<HistoryTick>();

        let mut rpg = Self {
            app,
            network: MockNetwork::default(),
            accepted_inputs: Vec::new(),
            seen_sequences: BTreeSet::new(),
            snapshots: BTreeMap::new(),
            current_tick: 0,
            retention_ticks,
            corrections: Vec::new(),
            rejected: Vec::new(),
        };
        rpg.spawn_combatant(ALICE, 100, Vec3i::new(0, 0, 0));
        rpg.spawn_combatant(BOB, 100, Vec3i::new(4, 0, 0));
        rpg.spawn_combatant(CAROL, 100, Vec3i::new(20, 0, 0));
        rpg.save_snapshot(0);
        rpg.set_history_tick(0);
        rpg
    }

    pub fn send(&mut self, input: ClientInput, latency_ticks: u32) {
        self.network.send(input, latency_ticks);
    }

    pub fn duplicate(&mut self, input: ClientInput, first_latency: u32, second_latency: u32) {
        self.send(input.clone(), first_latency);
        self.send(input, second_latency);
    }

    pub fn drop_input(&mut self, _input: ClientInput) {}

    pub fn receive_network_input(&mut self, input: ClientInput) {
        let replay_from = self.accept_input(input);
        self.sort_inputs();
        if let Some(replay_from) = replay_from {
            self.replay_from(replay_from);
        }
    }
    pub fn advance_to(&mut self, target_tick: u32) {
        for tick in self.current_tick.saturating_add(1)..=target_tick {
            if let Some(replay_from) = self.deliver_inputs(tick) {
                self.replay_from(replay_from);
            }
            self.simulate_tick(tick);
            self.current_tick = tick;
            self.save_snapshot(tick);
            self.set_history_tick(tick);
            self.prune_snapshots();
        }
    }

    pub fn hp(&mut self, stable_id: StableEntityId) -> i32 {
        self.combatant(stable_id).hp
    }

    pub fn position(&mut self, stable_id: StableEntityId) -> Vec3i {
        self.combatant(stable_id).position
    }

    pub fn facts(&self) -> &[CombatFact] {
        &self.app.world().resource::<CombatLog>().facts
    }

    pub fn corrections(&self) -> &[Correction] {
        &self.corrections
    }

    pub fn rejected(&self) -> &[RejectedInput] {
        &self.rejected
    }

    pub fn snapshot(&mut self) -> CombatSnapshot {
        capture_snapshot(&mut self.app)
    }

    pub fn death_markers_for(&mut self, victim: StableEntityId) -> usize {
        sorted_components::<DeathMarker>(&mut self.app)
            .into_iter()
            .filter(|(_, marker)| marker.victim == victim)
            .count()
    }

    pub fn corpses_for(&mut self, victim: StableEntityId) -> usize {
        sorted_components::<Corpse>(&mut self.app)
            .into_iter()
            .filter(|(_, corpse)| corpse.victim == victim)
            .count()
    }

    pub fn loot_for(&mut self, owner: StableEntityId) -> usize {
        sorted_components::<Loot>(&mut self.app)
            .into_iter()
            .filter(|(_, loot)| loot.owner == owner)
            .count()
    }

    pub fn inventory_food(&mut self, player: StableEntityId) -> u32 {
        sorted_components::<Inventory>(&mut self.app)
            .into_iter()
            .find_map(|(id, inventory)| (id == player).then_some(inventory.food))
            .unwrap_or_default()
    }

    pub fn projectile_count(&mut self) -> usize {
        sorted_components::<Projectile>(&mut self.app).len()
    }

    pub fn has_afterglow_network_resources(&self) -> bool {
        self.app
            .world()
            .contains_resource::<AfterglowLightyearConfig>()
            && self.app.world().contains_resource::<HistoryTick>()
    }

    fn deliver_inputs(&mut self, server_tick: u32) -> Option<u32> {
        let mut earliest_late = None;
        let retention_floor = self.current_tick.saturating_sub(self.retention_ticks);
        let delivered = self.network.deliver(server_tick);
        for input in delivered {
            if let Some(late_tick) = self.accept_input_with_floor(input, retention_floor) {
                earliest_late =
                    Some(earliest_late.map_or(late_tick, |tick: u32| tick.min(late_tick)));
            }
        }
        self.sort_inputs();
        earliest_late
    }

    fn accept_input(&mut self, input: ClientInput) -> Option<u32> {
        let retention_floor = self.current_tick.saturating_sub(self.retention_ticks);
        self.accept_input_with_floor(input, retention_floor)
    }

    fn accept_input_with_floor(&mut self, input: ClientInput, retention_floor: u32) -> Option<u32> {
        let late_tick = (input.tick <= self.current_tick).then_some(input.tick);
        let key = (input.player, input.sequence);
        if !self.seen_sequences.insert(key) {
            self.rejected.push(RejectedInput::Duplicate {
                player: input.player,
                sequence: input.sequence,
            });
            return None;
        }
        if input.tick < retention_floor {
            self.rejected.push(RejectedInput::Stale {
                player: input.player,
                sequence: input.sequence,
                tick: input.tick,
            });
            return None;
        }
        self.accepted_inputs.push(input);
        late_tick
    }

    fn sort_inputs(&mut self) {
        self.accepted_inputs
            .sort_by_key(|input| (input.tick, input.player, input.sequence));
    }

    fn replay_from(&mut self, tick: u32) {
        let anchor = tick.saturating_sub(1);
        let before = capture_snapshot(&mut self.app);
        let Some(snapshot) = self.snapshots.get(&anchor).cloned() else {
            return;
        };
        restore_snapshot(&mut self.app, snapshot);
        for replay_tick in anchor.saturating_add(1)..=self.current_tick {
            self.simulate_tick(replay_tick);
            self.save_snapshot(replay_tick);
            self.set_history_tick(replay_tick);
        }
        let after = capture_snapshot(&mut self.app);
        self.corrections.extend(diff_snapshots(&before, &after));
    }

    fn simulate_tick(&mut self, tick: u32) {
        self.apply_inputs(tick);
        self.resolve_projectiles(tick);
    }

    fn apply_inputs(&mut self, tick: u32) {
        let inputs = self
            .accepted_inputs
            .iter()
            .filter(|input| input.tick == tick)
            .cloned()
            .collect::<Vec<_>>();
        for input in inputs {
            match input.action {
                RpgAction::MoveTo(target) => self.apply_move(tick, input.player, target),
                RpgAction::RaiseShield => self.raise_shield(tick, input.player),
                RpgAction::AttackPrimary { target, damage } => {
                    self.cast_spell(tick, input.player, target, damage);
                }
                RpgAction::PickUpFood { from } => self.pick_up_food(tick, input.player, from),
            }
        }
    }

    fn apply_move(&mut self, tick: u32, player: StableEntityId, target: Vec3i) {
        let Some(entity) = self.entity_with::<Combatant>(player) else {
            return;
        };
        let current = self
            .app
            .world()
            .entity(entity)
            .get::<Combatant>()
            .unwrap()
            .position;
        let fact = if valid_move(current, target) {
            self.app
                .world_mut()
                .entity_mut(entity)
                .get_mut::<Combatant>()
                .unwrap()
                .position = target;
            CombatFact::MoveAccepted { tick, player }
        } else {
            CombatFact::MoveRejected { tick, player }
        };
        self.app
            .world_mut()
            .resource_mut::<CombatLog>()
            .facts
            .push(fact);
    }

    fn raise_shield(&mut self, tick: u32, player: StableEntityId) {
        let Some(entity) = self.entity_with::<Combatant>(player) else {
            return;
        };
        self.app
            .world_mut()
            .entity_mut(entity)
            .get_mut::<Combatant>()
            .unwrap()
            .shield_through = tick.saturating_add(1);
        self.app
            .world_mut()
            .resource_mut::<CombatLog>()
            .facts
            .push(CombatFact::ShieldRaised { tick, player });
    }

    fn cast_spell(
        &mut self,
        tick: u32,
        caster: StableEntityId,
        target: StableEntityId,
        damage: i32,
    ) {
        let Some(caster_entity) = self.entity_with::<Combatant>(caster) else {
            return;
        };
        let Some(target_entity) = self.entity_with::<Combatant>(target) else {
            return;
        };
        let caster_pos = self
            .app
            .world()
            .entity(caster_entity)
            .get::<Combatant>()
            .unwrap()
            .position;
        let target_pos = self
            .app
            .world()
            .entity(target_entity)
            .get::<Combatant>()
            .unwrap()
            .position;
        if !in_reach(caster_pos, target_pos) {
            self.app.world_mut().resource_mut::<CombatLog>().facts.push(
                CombatFact::SpellRejectedOutOfRange {
                    tick,
                    caster,
                    target,
                },
            );
            return;
        }
        self.spawn_projectile(
            projectile_id(tick, caster, target),
            Projectile {
                caster,
                target,
                impact_tick: tick.saturating_add(1),
                damage,
                resolved: false,
            },
        );
        self.app
            .world_mut()
            .resource_mut::<CombatLog>()
            .facts
            .push(CombatFact::SpellCast {
                tick,
                caster,
                target,
            });
    }

    fn pick_up_food(&mut self, tick: u32, player: StableEntityId, from: StableEntityId) {
        let Some(player_entity) = self.entity_with::<Inventory>(player) else {
            return;
        };
        let Some(loot_entity) = self.entity_with::<Loot>(from) else {
            return;
        };
        let loot = self
            .app
            .world()
            .entity(loot_entity)
            .get::<Loot>()
            .unwrap()
            .clone();
        if loot.picked_by.is_some() || loot.item != Item::Food {
            return;
        }
        self.app
            .world_mut()
            .entity_mut(loot_entity)
            .get_mut::<Loot>()
            .unwrap()
            .picked_by = Some(player);
        let mut entity = self.app.world_mut().entity_mut(player_entity);
        let mut inventory = entity.get_mut::<Inventory>().unwrap();
        inventory.food = inventory.food.saturating_add(1);
        self.app
            .world_mut()
            .resource_mut::<CombatLog>()
            .facts
            .push(CombatFact::FoodPickedUp { tick, player, from });
    }

    fn resolve_projectiles(&mut self, tick: u32) {
        for (entity, projectile) in self.unresolved_impacts(tick) {
            self.app
                .world_mut()
                .entity_mut(entity)
                .get_mut::<Projectile>()
                .unwrap()
                .resolved = true;
            let Some(target_entity) = self.entity_with::<Combatant>(projectile.target) else {
                continue;
            };
            let blocked = self
                .app
                .world()
                .entity(target_entity)
                .get::<Combatant>()
                .unwrap()
                .shield_through
                >= tick;
            if blocked {
                self.app.world_mut().resource_mut::<CombatLog>().facts.push(
                    CombatFact::SpellBlocked {
                        tick,
                        target: projectile.target,
                    },
                );
                continue;
            }
            let hp = {
                let mut entity = self.app.world_mut().entity_mut(target_entity);
                let mut target = entity.get_mut::<Combatant>().unwrap();
                target.hp = (target.hp - projectile.damage).max(0);
                target.hp
            };
            if hp == 0 && self.death_markers_for(projectile.target) == 0 {
                self.spawn_death_outputs(tick, projectile.target);
            }
        }
    }

    fn spawn_death_outputs(&mut self, tick: u32, victim: StableEntityId) {
        self.app
            .world_mut()
            .spawn((death_marker_id(victim), DeathMarker { victim }));
        self.app
            .world_mut()
            .spawn((corpse_id(victim), Corpse { victim }));
        self.app.world_mut().spawn((
            loot_id(victim),
            Loot {
                owner: victim,
                item: Item::Food,
                picked_by: None,
            },
        ));
        self.app
            .world_mut()
            .resource_mut::<CombatLog>()
            .facts
            .push(CombatFact::PlayerDied {
                tick,
                player: victim,
            });
    }

    fn spawn_combatant(&mut self, stable: StableEntityId, hp: i32, position: Vec3i) {
        self.app
            .world_mut()
            .spawn((stable, Combatant::new(hp, position), Inventory::default()));
    }

    fn spawn_projectile(&mut self, stable: StableEntityId, projectile: Projectile) {
        self.app.world_mut().spawn((stable, projectile));
    }

    fn save_snapshot(&mut self, tick: u32) {
        let snapshot = capture_snapshot(&mut self.app);
        self.snapshots.insert(tick, snapshot);
    }

    fn prune_snapshots(&mut self) {
        let floor = retained_snapshot_floor(self.current_tick, self.retention_ticks);
        self.snapshots.retain(|tick, _| *tick >= floor);
    }

    fn set_history_tick(&mut self, tick: u32) {
        self.app.world_mut().resource_mut::<HistoryTick>().0 = tick;
    }

    fn combatant(&mut self, stable: StableEntityId) -> Combatant {
        sorted_components::<Combatant>(&mut self.app)
            .into_iter()
            .find_map(|(id, combatant)| (id == stable).then_some(combatant))
            .expect("combatant should exist")
    }

    fn entity_with<T: Component>(&mut self, stable: StableEntityId) -> Option<Entity> {
        let mut query = self
            .app
            .world_mut()
            .query_filtered::<(Entity, &StableEntityId), With<T>>();
        query
            .iter(self.app.world())
            .find_map(|(entity, id)| (*id == stable).then_some(entity))
    }

    fn unresolved_impacts(&mut self, tick: u32) -> Vec<(Entity, Projectile)> {
        let mut query = self.app.world_mut().query::<(Entity, &Projectile)>();
        query
            .iter(self.app.world())
            .filter(|(_, projectile)| !projectile.resolved && projectile.impact_tick <= tick)
            .map(|(entity, projectile)| (entity, projectile.clone()))
            .collect()
    }
}
