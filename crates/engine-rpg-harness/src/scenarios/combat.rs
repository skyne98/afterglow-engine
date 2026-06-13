use super::{components::*, systems::*};
use crate::rig::LightyearTestRig;
use afterglow_engine::{
    controller::{
        AfterglowFirstPersonControllerPlugin, ControllerStance, FirstPersonController,
        FirstPersonControllerConfig, FirstPersonEffectStack, FirstPersonImpulseBuffer,
    },
    core::identity::StableEntityId,
    input::AfterglowAction,
    network::{LightyearRole, register_afterglow_lightyear_protocol},
    physics::AfterglowPhysicsPlugin,
};
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::*;

// ── Entity IDs ──────────────────────────────────────────────────────────────
const ALICE: StableEntityId = StableEntityId::from_raw(1);
const BOB: StableEntityId = StableEntityId::from_raw(2);
const CAROL: StableEntityId = StableEntityId::from_raw(3);
const ENEMY_ID: StableEntityId = StableEntityId::from_raw(100);
const BOSS_ID: StableEntityId = StableEntityId::from_raw(200);

// ── Helpers ─────────────────────────────────────────────────────────────────

fn reconcile_controller_components(
    mut effects: Query<
        (
            &mut FirstPersonEffectStack,
            &Confirmed<FirstPersonEffectStack>,
        ),
        With<Predicted>,
    >,
    mut impulses: Query<
        (
            &mut FirstPersonImpulseBuffer,
            &Confirmed<FirstPersonImpulseBuffer>,
        ),
        With<Predicted>,
    >,
) {
    for (mut predicted, confirmed) in &mut effects {
        if *predicted != confirmed.0 {
            *predicted = confirmed.0.clone();
        }
    }
    for (mut predicted, confirmed) in &mut impulses {
        if *predicted != confirmed.0 {
            *predicted = confirmed.0;
        }
    }
}

fn register_combat(app: &mut App, _role: LightyearRole) {
    register_afterglow_lightyear_protocol(app);
    app.init_resource::<AttackCooldown>();
    app.register_component::<Health>().add_prediction();
    app.register_component::<CombatState>().add_prediction();
    app.register_component::<FirstPersonEffectStack>()
        .add_prediction();
    app.register_component::<FirstPersonImpulseBuffer>()
        .add_prediction();
    app.register_component::<Corpse>();
    app.register_component::<Loot>().add_prediction();
    app.register_component::<SpawnPoint>();
    app.register_component::<DeadTimer>();
    app.register_component::<Team>();
    app.register_component::<Enemy>();
    app.register_component::<Boss>();
    app.add_systems(
        FixedUpdate,
        (
            advance_history_tick,
            sync_dead_state,
            resolve_shields,
            enemy_attack_system,
            resolve_attacks,
            resolve_aoe_attacks,
            apply_deaths,
            mark_dead_for_respawn,
            respawn_dead_players,
            boss_phase_transition,
        )
            .chain(),
    );
    app.add_systems(
        PreUpdate,
        reconcile_controller_components.after(ReplicationSystems::Receive),
    );
}

fn player_bundle(pos: Vec3) -> impl Bundle {
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    (
        Health {
            current: 100,
            max: 100,
        },
        CombatState::default(),
        FirstPersonController { config },
        Transform::from_translation(pos + Vec3::Y * half_height),
        ActionState::<AfterglowAction>::default(),
    )
}

fn set_action_state(world: &mut World, entity: Entity, action: AfterglowAction) {
    let mut state = ActionState::<AfterglowAction>::default();
    state.press(&action);
    world.entity_mut(entity).insert(state);
}

fn clear_action_state(world: &mut World, entity: Entity) {
    world
        .entity_mut(entity)
        .insert(ActionState::<AfterglowAction>::default());
}

fn setup_multiplayer_rig(client_count: usize, with_physics: bool) -> LightyearTestRig {
    LightyearTestRig::new(
        client_count,
        |app| {
            if with_physics {
                app.add_plugins(AfterglowPhysicsPlugin);
            }
            app.add_plugins(AfterglowFirstPersonControllerPlugin);
        },
        register_combat,
    )
    .with_input_delay_ms(50)
}

fn spawn_player(rig: &mut LightyearTestRig, sid: StableEntityId, pos: Vec3, team: Team) -> Entity {
    let entity = rig.spawn_replicated(sid, (player_bundle(pos), team));
    let mut entities = vec![entity];
    for i in 0..rig.client_apps.len() {
        let c = rig
            .find_client_entity(i, sid)
            .unwrap_or_else(|| panic!("client {i} entity for {sid:?}"));
        entities.push(c);
    }
    rig.register_entity(sid, entities);
    entity
}

fn spawn_enemy(
    rig: &mut LightyearTestRig,
    sid: StableEntityId,
    pos: Vec3,
    hp: i32,
    spawn_pos: Option<Vec3>,
) -> Entity {
    let entity = rig
        .server_app
        .world_mut()
        .spawn((
            Health {
                current: hp,
                max: hp,
            },
            CombatState::default(),
            Transform::from_translation(pos),
            sid,
            ActionState::<AfterglowAction>::default(),
            Enemy {
                attack_range: 3.0,
                damage: 10,
                detection_range: 10.0,
            },
            SpawnPoint {
                position: spawn_pos.unwrap_or(pos),
            },
        ))
        .id();
    rig.server_app.world_mut().run_schedule(PostUpdate);
    entity
}

fn spawn_boss(
    rig: &mut LightyearTestRig,
    sid: StableEntityId,
    pos: Vec3,
    hp: i32,
    thresholds: Vec<i32>,
) -> Entity {
    let entity = rig
        .server_app
        .world_mut()
        .spawn((
            Health {
                current: hp,
                max: hp,
            },
            CombatState::default(),
            Transform::from_translation(pos),
            sid,
            ActionState::<AfterglowAction>::default(),
            Team(2),
            Boss {
                phase: 1,
                max_phases: (thresholds.len() + 1) as u32,
                phase_hp_thresholds: thresholds,
            },
        ))
        .id();
    rig.server_app.world_mut().run_schedule(PostUpdate);
    entity
}

/// Delivery tick for actions queued at tick 1 with 50ms delay (~3 ticks).
const DELIVERY_TICK: u32 = 4;

// ── Shared scenario runner ──────────────────────────────────────────────────

pub fn run_combat_scenarios(_rig: &mut LightyearTestRig) {}

mod pve;
mod pvp;
