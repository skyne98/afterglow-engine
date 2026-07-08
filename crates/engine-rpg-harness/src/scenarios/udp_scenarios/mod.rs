pub mod adversarial;
pub mod combat;
pub mod doors;
pub mod full_stack;
pub mod lockstep;
pub mod multiplayer_boxes;
pub mod multiplayer_boxes_live_rope;
pub mod multiplayer_boxes_rope;
pub mod native_input;
pub mod prespawned;
pub mod rpg;
pub mod stress;

use crate::{TransportConfig, rig::LightyearTestRig};
use afterglow_engine::{
    core::identity::StableEntityId,
    input::AfterglowAction,
    network::{HistoryTick, LightyearRole, register_afterglow_lightyear_protocol},
    physics::{
        AfterglowPhysicsPlugin,
        avian::{Collider, Gravity, RigidBody},
    },
};
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

use super::{components::*, systems::*};

const ALICE: StableEntityId = StableEntityId::from_raw(1);
const BOB: StableEntityId = StableEntityId::from_raw(2);
const ENEMY_ID: StableEntityId = StableEntityId::from_raw(100);
const HEAL_AMOUNT: i32 = 10;
const UDP_STRESS_ENTITY_COUNT: usize = 50;
const DOOR: StableEntityId = StableEntityId::from_raw(1);
const PLAYER: StableEntityId = StableEntityId::from_raw(2);

#[derive(Component)]
struct HealApplied;

#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct TestCue {
    value: u32,
}

fn register_lockstep(app: &mut App, _role: LightyearRole) {
    register_afterglow_lightyear_protocol(app);
    app.register_component::<Health>().add_prediction();
    app.register_component::<CombatState>().add_prediction();
    app.add_systems(
        FixedUpdate,
        (advance_history_tick, resolve_shields, resolve_attacks).chain(),
    );
}

fn register_adversarial(app: &mut App, _role: LightyearRole) {
    register_afterglow_lightyear_protocol(app);
    app.register_component::<Health>().add_prediction();
    app.register_component::<CombatState>().add_prediction();
    app.add_systems(
        FixedUpdate,
        (advance_history_tick, resolve_attacks, move_players).chain(),
    );
}

fn register_rpg(app: &mut App, _role: LightyearRole) {
    register_afterglow_lightyear_protocol(app);
    app.register_component::<Health>().add_prediction();
    app.register_component::<ManaPool>().add_prediction();
    app.register_component::<CombatState>().add_prediction();
    app.register_component::<Corpse>();
    app.register_component::<Loot>().add_prediction();
    app.register_component::<BurnEffect>().add_prediction();
    app.register_component::<SpawnPoint>().add_prediction();
    app.register_component::<Transform>().add_prediction();
    app.register_component::<DeadTimer>().add_prediction();
    app.add_systems(
        FixedUpdate,
        (
            advance_history_tick,
            process_mana_for_attack,
            resolve_shields,
            resolve_attacks,
            resolve_aoe_attacks,
            apply_burn_damage,
            apply_deaths,
            mark_dead_for_respawn,
            respawn_dead_players,
            resolve_loot_pickup,
            move_players,
        )
            .chain(),
    );
}

fn register_stress(app: &mut App, _role: LightyearRole) {
    register_afterglow_lightyear_protocol(app);
    app.register_component::<Health>().add_prediction();
    app.register_component::<CombatState>().add_prediction();
}

fn register_test_protocol(app: &mut App, _role: LightyearRole) {
    register_afterglow_lightyear_protocol(app);
    app.register_component::<TestCue>().add_prediction();
}

fn register_drift_protocol(app: &mut App, _role: LightyearRole) {
    register_afterglow_lightyear_protocol(app);
    app.register_component::<Health>().add_prediction();
    app.add_systems(
        PreUpdate,
        reconcile_client_health.after(ReplicationSystems::Receive),
    );
}

fn register_doors(app: &mut App, _role: LightyearRole) {
    register_afterglow_lightyear_protocol(app);
    app.register_component::<Health>().add_prediction();
    app.register_component::<DoorState>().add_prediction();
    app.register_component::<DoorGrab>().add_prediction();
    app.add_systems(
        FixedUpdate,
        (resolve_door_interactions, apply_door_open).chain(),
    );
}

fn register_combat(app: &mut App, _role: LightyearRole) {
    register_afterglow_lightyear_protocol(app);
    app.register_component::<Health>().add_prediction();
    app.register_component::<CombatState>().add_prediction();
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
}

fn reconcile_client_health(mut query: Query<(&mut Health, &Confirmed<Health>), With<Predicted>>) {
    for (mut predicted, confirmed) in &mut query {
        if *predicted != confirmed.0 {
            *predicted = confirmed.0;
        }
    }
}

fn lockstep_player(pos: Vec3) -> impl Bundle {
    (
        Health {
            current: 100,
            max: 200,
        },
        CombatState::default(),
        Transform::from_translation(pos),
        ActionState::<AfterglowAction>::default(),
    )
}

fn rpg_player(pos: Vec3) -> impl Bundle {
    (
        Health {
            current: 100,
            max: 100,
        },
        ManaPool {
            current: 100,
            max: 100,
        },
        CombatState::default(),
        SpawnPoint { position: pos },
        Transform::from_translation(pos),
        ActionState::<AfterglowAction>::default(),
    )
}

fn set_action(world: &mut World, entity: Entity, action: AfterglowAction) {
    let mut state = ActionState::<AfterglowAction>::default();
    state.press(&action);
    world.entity_mut(entity).insert(state);
}

fn clear_action(world: &mut World, entity: Entity) {
    world
        .entity_mut(entity)
        .insert(ActionState::<AfterglowAction>::default());
}

fn udp_rig(client_count: usize, register: impl Fn(&mut App, LightyearRole)) -> LightyearTestRig {
    let mut rig = LightyearTestRig::new_with_transport(
        client_count,
        |_| {},
        register,
        TransportConfig::Udp { server_port: 0 },
    );
    rig.connect();
    rig
}

fn spawn_enemy(rig: &mut LightyearTestRig, sid: StableEntityId, pos: Vec3, hp: i32) -> Entity {
    rig.server_app
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
            SpawnPoint { position: pos },
        ))
        .id()
}

fn door_player(pos: Vec3) -> impl Bundle {
    (
        Health {
            current: 100,
            max: 100,
        },
        Transform::from_translation(pos),
        ActionState::<AfterglowAction>::default(),
        RigidBody::Dynamic,
        Collider::sphere(0.3),
    )
}

fn door_bundle(pos: Vec3, open: bool, locked: bool) -> impl Bundle {
    (
        DoorState { open, locked },
        Transform::from_translation(pos),
        RigidBody::Kinematic,
        Collider::cuboid(0.5, 1.0, 0.05),
    )
}

fn zero_gravity(app: &mut App) {
    if let Some(mut gravity) = app.world_mut().get_resource_mut::<Gravity>() {
        gravity.0 = Vec3::ZERO;
    }
}

fn door_grab_hash(player: StableEntityId, door: StableEntityId) -> u64 {
    (player.0 ^ door.0) as u64
}

fn udp_combat_rig(client_count: usize) -> LightyearTestRig {
    let mut rig = LightyearTestRig::new_with_transport(
        client_count,
        |app| {
            app.add_plugins(AfterglowPhysicsPlugin);
        },
        register_combat,
        TransportConfig::Udp { server_port: 0 },
    );
    rig.connect();
    rig.with_input_delay_ms(50)
}

fn combat_spawn_player(
    rig: &mut LightyearTestRig,
    sid: StableEntityId,
    pos: Vec3,
    team: Team,
) -> Entity {
    let entity = rig.spawn_replicated(
        sid,
        (
            Health {
                current: 100,
                max: 100,
            },
            CombatState::default(),
            Transform::from_translation(pos),
            ActionState::<AfterglowAction>::default(),
            team,
        ),
    );
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
