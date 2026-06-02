use afterglow_engine::{
    controller::FirstPersonController,
    core::identity::StableEntityId,
    input::AfterglowAction,
    network::{AfterglowLightyearConfig, HistoryTick, LightyearRole},
};
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::{NetworkTarget, PreSpawned, PredictionTarget, Replicate};

use super::components::*;

const SHIELD_DURATION: u32 = 20;
const ATTACK_RANGE: f32 = 8.0;
const LOOT_PICKUP_RANGE: f32 = 2.0;
pub const ATTACK_DAMAGE: i32 = 34;
const AOE_RANGE: f32 = 10.0;
pub const AOE_DAMAGE: i32 = 25;
pub const ATTACK_COOLDOWN_TICKS: u32 = 3;
pub const KNOCKBACK_FORCE: f32 = 2.0;

#[derive(Resource)]
pub struct AttackCooldown(pub u32);

impl Default for AttackCooldown {
    fn default() -> Self {
        Self(0)
    }
}

pub fn advance_history_tick(mut tick: ResMut<HistoryTick>) {
    tick.0 = tick.0.wrapping_add(1);
}

pub fn resolve_attacks(
    players: Query<
        (
            Entity,
            &StableEntityId,
            &ActionState<AfterglowAction>,
            Option<&Team>,
        ),
        (With<Health>, With<Transform>),
    >,
    mut health_query: Query<&mut Health>,
    mut combat_query: Query<&mut CombatState>,
    mut transforms: Query<&mut Transform>,
    tick: Res<HistoryTick>,
    cooldown: Option<Res<AttackCooldown>>,
) {
    let cd = cooldown.as_ref().map_or(0, |r| r.0);
    let mut attackers: Vec<(Entity, Vec3, Option<Team>)> = Vec::new();
    for (entity, _sid, action, team) in players.iter() {
        if !action.pressed(&AfterglowAction::AttackPrimary) {
            continue;
        }
        if let Ok(combat) = combat_query.get(entity) {
            if combat.dead {
                continue;
            }
            if cd > 0 && combat.last_attack_tick + cd > tick.0 {
                continue;
            }
        }
        let pos = transforms
            .get(entity)
            .map(|t| t.translation)
            .unwrap_or_default();
        attackers.push((entity, pos, team.copied()));
    }

    for (attacker, attacker_pos, attacker_team) in attackers {
        let mut best_target: Option<Entity> = None;
        let mut best_dist = f32::MAX;

        for (other, _other_sid, _action, other_team) in players.iter() {
            if other == attacker {
                continue;
            }
            if let (Some(at), Some(ot)) = (&attacker_team, other_team) {
                if at.0 == ot.0 {
                    continue;
                }
            }
            let other_pos = transforms
                .get(other)
                .map(|t| t.translation)
                .unwrap_or_default();
            let dist = other_pos.distance(attacker_pos);
            if dist > ATTACK_RANGE || dist >= best_dist {
                continue;
            }
            if let Ok(h) = health_query.get(other) {
                if h.current <= 0 {
                    continue;
                }
            } else {
                continue;
            }
            best_dist = dist;
            best_target = Some(other);
        }

        if let Some(target) = best_target {
            let blocked = combat_query
                .get(target)
                .map_or(false, |c| c.shield_active_until > tick.0 && !c.dead);

            if blocked {
                continue;
            }

            if let Ok(mut h) = health_query.get_mut(target) {
                h.current = (h.current - ATTACK_DAMAGE).max(0);
            }
            if let Ok(mut c) = combat_query.get_mut(attacker) {
                c.last_attack_tick = tick.0;
            }
            if let Ok(mut t) = transforms.get_mut(target) {
                let dir = (t.translation - attacker_pos).normalize_or_zero();
                t.translation += dir * KNOCKBACK_FORCE;
            }
        }
    }
}

pub fn resolve_aoe_attacks(
    players: Query<
        (Entity, &ActionState<AfterglowAction>, Option<&Team>),
        (With<Health>, With<Transform>),
    >,
    mut health_query: Query<&mut Health>,
    mut combat_query: Query<&mut CombatState>,
    transforms: Query<&Transform>,
    tick: Res<HistoryTick>,
) {
    let mut attackers: Vec<(Entity, Vec3, Option<Team>)> = Vec::new();
    for (entity, action, team) in players.iter() {
        if !action.pressed(&AfterglowAction::AttackSecondary) {
            continue;
        }
        if let Ok(combat) = combat_query.get(entity) {
            if combat.dead {
                continue;
            }
        }
        let pos = transforms
            .get(entity)
            .map(|t| t.translation)
            .unwrap_or_default();
        attackers.push((entity, pos, team.copied()));
    }

    for (attacker, attacker_pos, attacker_team) in attackers {
        for (other, _action, other_team) in players.iter() {
            if other == attacker {
                continue;
            }
            if let (Some(at), Some(ot)) = (&attacker_team, other_team) {
                if at.0 == ot.0 {
                    continue;
                }
            }
            let other_pos = transforms
                .get(other)
                .map(|t| t.translation)
                .unwrap_or_default();
            let dist = other_pos.distance(attacker_pos);
            if dist > AOE_RANGE {
                continue;
            }
            if let Ok(h) = health_query.get(other) {
                if h.current <= 0 {
                    continue;
                }
            } else {
                continue;
            }
            let blocked = combat_query
                .get(other)
                .map_or(false, |c| c.shield_active_until > tick.0 && !c.dead);
            if blocked {
                continue;
            }
            if let Ok(mut h) = health_query.get_mut(other) {
                h.current = (h.current - AOE_DAMAGE).max(0);
            }
            if let Ok(mut c) = combat_query.get_mut(attacker) {
                c.last_attack_tick = tick.0;
            }
        }
    }
}

pub fn resolve_shields(
    mut players: Query<(&ActionState<AfterglowAction>, &mut CombatState)>,
    tick: Res<HistoryTick>,
) {
    for (action, mut combat) in players.iter_mut() {
        if !action.pressed(&AfterglowAction::RaiseShield) {
            continue;
        }
        if combat.dead {
            continue;
        }
        if combat.shield_active_until > tick.0 {
            continue;
        }
        combat.shield_active_until = tick.0 + SHIELD_DURATION;
    }
}

pub fn sync_dead_state(
    mut commands: Commands,
    query: Query<(Entity, &CombatState, Option<&FirstPersonController>), Changed<CombatState>>,
) {
    for (entity, combat, controller) in &query {
        if combat.dead && controller.is_some() {
            commands.entity(entity).remove::<FirstPersonController>();
        } else if !combat.dead && controller.is_none() {
            commands.entity(entity).insert(FirstPersonController::new());
        }
    }
}

pub fn apply_deaths(
    mut commands: Commands,
    mut query: Query<(Entity, &StableEntityId, &Health, &mut CombatState)>,
    config: Option<Res<AfterglowLightyearConfig>>,
) {
    let is_server = config.map_or(false, |c| c.role == LightyearRole::Server);
    for (_entity, sid, health, mut combat) in query.iter_mut() {
        if health.current > 0 || combat.dead {
            continue;
        }
        combat.dead = true;
        if is_server {
            commands.spawn((Corpse { victim: *sid },));
            commands.spawn((Loot {
                owner: *sid,
                picked_up: false,
            },));
        }
    }
}

pub fn resolve_loot_pickup(
    players: Query<(&ActionState<AfterglowAction>, &Transform)>,
    mut loot_query: Query<(&mut Loot, &Transform)>,
    _tick: Res<HistoryTick>,
) {
    for (action, player_transform) in players.iter() {
        if !action.pressed(&AfterglowAction::Use) {
            continue;
        }
        let player_pos = player_transform.translation;
        for (mut loot, loot_transform) in loot_query.iter_mut() {
            if loot.picked_up {
                continue;
            }
            let dist = loot_transform.translation.distance(player_pos);
            if dist <= LOOT_PICKUP_RANGE {
                loot.picked_up = true;
            }
        }
    }
}

pub fn process_mana_for_attack(
    mut query: Query<(&mut ActionState<AfterglowAction>, &mut ManaPool)>,
) {
    for (mut action, mut mana) in query.iter_mut() {
        if action.pressed(&AfterglowAction::AttackPrimary) {
            if mana.current >= 30 {
                mana.current -= 30;
            } else {
                *action = ActionState::default();
            }
        }
    }
}

pub fn apply_burn_damage(
    mut commands: Commands,
    mut query: Query<(Entity, &mut Health, &mut BurnEffect)>,
) {
    for (entity, mut health, mut burn) in query.iter_mut() {
        health.current = (health.current - burn.damage_per_tick).max(0);
        burn.remaining_ticks = burn.remaining_ticks.saturating_sub(1);
        if burn.remaining_ticks == 0 {
            commands.entity(entity).remove::<BurnEffect>();
        }
    }
}

pub fn mark_dead_for_respawn(
    mut commands: Commands,
    query: Query<(Entity, &CombatState), Without<DeadTimer>>,
) {
    for (entity, combat) in query.iter() {
        if combat.dead {
            commands.entity(entity).insert(DeadTimer { remaining: 10 });
        }
    }
}

pub fn respawn_dead_players(
    mut commands: Commands,
    mut query: Query<(
        Entity,
        &mut Health,
        &mut CombatState,
        &mut DeadTimer,
        &SpawnPoint,
    )>,
) {
    for (entity, mut health, mut combat, mut timer, spawn) in query.iter_mut() {
        if !combat.dead {
            continue;
        }
        if timer.remaining > 0 {
            timer.remaining -= 1;
            continue;
        }
        health.current = health.max;
        combat.dead = false;
        commands
            .entity(entity)
            .insert(Transform::from_translation(spawn.position));
        commands.entity(entity).remove::<DeadTimer>();
    }
}

pub fn move_players(mut query: Query<(&ActionState<AfterglowAction>, &mut Transform)>) {
    let speed = 5.0;
    let dt = 1.0 / 60.0;
    for (action, mut transform) in query.iter_mut() {
        let move_axis = action.clamped_axis_pair(&AfterglowAction::Move);
        if move_axis.length_squared() > 0.0 {
            transform.translation += Vec3::new(move_axis.x, 0.0, move_axis.y) * speed * dt;
        }
    }
}

pub fn door_grab_hash(player: StableEntityId, door: StableEntityId) -> u64 {
    (player.0 ^ door.0) as u64
}

pub fn resolve_door_interactions(
    mut commands: Commands,
    players: Query<(&StableEntityId, &ActionState<AfterglowAction>, &Transform)>,
    doors: Query<(&StableEntityId, &DoorState, &Transform)>,
    config: Option<Res<AfterglowLightyearConfig>>,
) {
    let is_server = config.map_or(false, |c| c.role == LightyearRole::Server);
    if !is_server {
        return;
    }
    for (player_sid, action, player_transform) in &players {
        if !action.pressed(&AfterglowAction::Use) {
            continue;
        }
        for (door_sid, door_state, door_transform) in &doors {
            if door_state.locked || door_state.open {
                continue;
            }
            let dist = player_transform
                .translation
                .distance(door_transform.translation);
            if dist > 3.0 {
                continue;
            }
            let hash = door_grab_hash(*player_sid, *door_sid);
            commands.spawn((
                DoorGrab {
                    player: *player_sid,
                    door: *door_sid,
                },
                PreSpawned::new(hash),
                Replicate::to_clients(NetworkTarget::All),
                PredictionTarget::to_clients(NetworkTarget::All),
            ));
        }
    }
}

pub fn enemy_attack_system(
    mut enemies: Query<(
        Entity,
        &Transform,
        &Enemy,
        &mut ActionState<AfterglowAction>,
    )>,
    players: Query<(&Transform, &Health), Without<Enemy>>,
) {
    for (_entity, transform, enemy, mut action) in enemies.iter_mut() {
        let mut found = false;
        for (player_transform, health) in players.iter() {
            if health.current <= 0 {
                continue;
            }
            let dist = transform.translation.distance(player_transform.translation);
            if dist <= enemy.detection_range {
                let mut state = ActionState::<AfterglowAction>::default();
                if dist <= enemy.attack_range {
                    state.press(&AfterglowAction::AttackPrimary);
                }
                *action = state;
                found = true;
                break;
            }
        }
        if !found {
            *action = ActionState::default();
        }
    }
}

pub fn boss_phase_transition(mut bosses: Query<(&Health, &mut Boss)>) {
    for (health, mut boss) in bosses.iter_mut() {
        let mut new_phase = 1u32;
        for (i, threshold) in boss.phase_hp_thresholds.iter().enumerate() {
            if health.current <= *threshold {
                new_phase = (i + 2) as u32;
            }
        }
        boss.phase = new_phase.clamp(1, boss.max_phases);
    }
}

pub fn apply_door_open(
    mut doors: Query<(&StableEntityId, &mut DoorState, &Transform), Without<Health>>,
    grabs: Query<&DoorGrab>,
    mut players: Query<(&StableEntityId, &mut Transform), With<Health>>,
) {
    for grab in &grabs {
        let mut door_pos = None;
        for (door_sid, mut door_state, door_transform) in &mut doors {
            if *door_sid == grab.door && !door_state.open {
                door_state.open = true;
                door_pos = Some(door_transform.translation);
            }
        }
        if let Some(door_pos) = door_pos {
            for (player_sid, mut player_transform) in &mut players {
                if *player_sid == grab.player {
                    let dir = (door_pos - player_transform.translation).normalize_or_zero();
                    player_transform.translation += dir * 0.1;
                }
            }
        }
    }
}
