use super::{components::*, systems::*};
use crate::rig::LightyearTestRig;
use afterglow_engine::{
    core::identity::StableEntityId,
    input::AfterglowAction,
    network::{HistoryTick, LightyearRole},
};
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::*;

const ALICE: StableEntityId = StableEntityId::from_raw(1);
const BOB: StableEntityId = StableEntityId::from_raw(2);
const CHARLIE: StableEntityId = StableEntityId::from_raw(3);
const DAVE: StableEntityId = StableEntityId::from_raw(4);

fn player_bundle(pos: Vec3) -> impl Bundle {
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

fn register_rpg(app: &mut App, _role: LightyearRole) {
    app.init_resource::<HistoryTick>();
    app.register_component::<StableEntityId>();
    app.register_component::<Health>().add_prediction();
    app.register_component::<ManaPool>().add_prediction();
    app.register_component::<CombatState>().add_prediction();
    app.register_component::<Corpse>();
    app.register_component::<Loot>().add_prediction();
    app.register_component::<BurnEffect>().add_prediction();
    app.register_component::<SpawnPoint>().add_prediction();
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

#[test]
fn networked_movement() {
    let mut rig = LightyearTestRig::new(1, |_| {}, register_rpg);

    let alice = rig.spawn_replicated(ALICE, player_bundle(Vec3::ZERO));
    let alice_c0 = rig.find_client_entity(0, ALICE).expect("ALICE on client 0");
    rig.client_world_mut(0)
        .entity_mut(alice_c0)
        .insert(Transform::from_translation(Vec3::ZERO));
    rig.register_entity(ALICE, vec![alice, alice_c0]);

    let mut state = ActionState::<AfterglowAction>::default();
    state.set_axis_pair(&AfterglowAction::Move, Vec2::new(0.0, 1.0));
    rig.client_world_mut(0).entity_mut(alice_c0).insert(state);

    rig.queue_action(1, {
        let alice = alice;
        move |app| {
            let mut s = ActionState::<AfterglowAction>::default();
            s.set_axis_pair(&AfterglowAction::Move, Vec2::new(0.0, 1.0));
            app.world_mut().entity_mut(alice).insert(s);
        }
    });

    let server_pos_before = rig
        .server_component::<Transform>(alice)
        .unwrap()
        .translation;

    rig.advance_to(15);

    let server_pos_after = rig
        .server_component::<Transform>(alice)
        .unwrap()
        .translation;
    let moved = server_pos_after.distance(server_pos_before);
    assert!(
        moved > 0.2,
        "Player should have moved at least 0.2 units after 15 ticks with move input"
    );

    let client_pos = rig
        .client_component::<Transform>(0, alice_c0)
        .unwrap()
        .translation;
    assert!(
        (server_pos_after - client_pos).length() < 0.01,
        "Client position should match server position"
    );
}

#[test]
fn death_respawn_cycle() {
    let mut rig = LightyearTestRig::new(1, |_| {}, register_rpg);

    let alice = rig.spawn_replicated(
        ALICE,
        (
            Health {
                current: 30,
                max: 100,
            },
            ManaPool {
                current: 100,
                max: 100,
            },
            CombatState::default(),
            SpawnPoint {
                position: Vec3::new(10.0, 0.0, 10.0),
            },
            Transform::from_translation(Vec3::new(10.0, 0.0, 10.0)),
            ActionState::<AfterglowAction>::default(),
        ),
    );
    let bob = rig.spawn_replicated(
        BOB,
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
            SpawnPoint {
                position: Vec3::new(5.0, 0.0, 10.0),
            },
            Transform::from_translation(Vec3::new(5.0, 0.0, 10.0)),
            ActionState::<AfterglowAction>::default(),
        ),
    );

    rig.queue_action(1, {
        let bob = bob;
        move |app| {
            let mut state = ActionState::<AfterglowAction>::default();
            state.press(&AfterglowAction::AttackPrimary);
            app.world_mut().entity_mut(bob).insert(state);
        }
    });
    rig.queue_action(2, {
        let bob = bob;
        move |app| {
            app.world_mut()
                .entity_mut(bob)
                .insert(ActionState::<AfterglowAction>::default());
        }
    });

    rig.advance_to(5);

    assert!(
        rig.server_component::<CombatState>(alice).unwrap().dead,
        "ALICE should be dead after BOB's attack"
    );
    assert_eq!(
        rig.server_component::<Health>(alice).unwrap().current,
        0,
        "ALICE's HP should be 0"
    );
    assert!(
        rig.server_component::<DeadTimer>(alice).is_some(),
        "ALICE should have a DeadTimer"
    );

    rig.advance_to(30);

    assert!(
        !rig.server_component::<CombatState>(alice).unwrap().dead,
        "ALICE should be respawned and alive"
    );
    assert_eq!(
        rig.server_component::<Health>(alice).unwrap().current,
        100,
        "ALICE should have full HP after respawn"
    );
    assert!(
        rig.server_component::<DeadTimer>(alice).is_none(),
        "DeadTimer should be removed after respawn"
    );
    let spawn_pos = rig.server_component::<SpawnPoint>(alice).unwrap().position;
    let server_pos = rig
        .server_component::<Transform>(alice)
        .unwrap()
        .translation;
    assert!(
        (server_pos - spawn_pos).length() < 0.1,
        "ALICE should be at spawn point after respawn"
    );
}

#[test]
fn status_effects_over_time() {
    let mut rig = LightyearTestRig::new(1, |_| {}, register_rpg);

    let alice = rig.spawn_replicated(
        ALICE,
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
            BurnEffect {
                remaining_ticks: 10,
                damage_per_tick: 5,
            },
            Transform::from_translation(Vec3::ZERO),
            ActionState::<AfterglowAction>::default(),
        ),
    );

    rig.advance(5);
    assert_eq!(
        rig.server_component::<Health>(alice).unwrap().current,
        75,
        "HP should be 75 after 5 ticks of burn (5 dmg/tick)"
    );

    rig.advance(5);
    assert_eq!(
        rig.server_component::<Health>(alice).unwrap().current,
        50,
        "HP should be 50 after 10 ticks of burn"
    );
    assert!(
        rig.server_component::<BurnEffect>(alice).is_none(),
        "BurnEffect should be removed after all ticks expire"
    );
}

#[test]
fn status_effects_can_kill() {
    let mut rig = LightyearTestRig::new(1, |_| {}, register_rpg);

    let alice = rig.spawn_replicated(
        ALICE,
        (
            Health {
                current: 20,
                max: 100,
            },
            ManaPool {
                current: 100,
                max: 100,
            },
            CombatState::default(),
            BurnEffect {
                remaining_ticks: 10,
                damage_per_tick: 5,
            },
            Transform::from_translation(Vec3::ZERO),
            ActionState::<AfterglowAction>::default(),
        ),
    );

    rig.advance(4);
    assert_eq!(
        rig.server_component::<Health>(alice).unwrap().current,
        0,
        "HP should reach 0 from burn before all ticks expire"
    );
    assert!(
        rig.server_component::<CombatState>(alice).unwrap().dead,
        "ALICE should be dead from burn damage"
    );
}

#[test]
fn cooldown_and_resource_cost() {
    let mut rig = LightyearTestRig::new(2, |_| {}, register_rpg);

    let alice = rig.spawn_replicated(
        ALICE,
        (
            Health {
                current: 100,
                max: 100,
            },
            ManaPool {
                current: 50,
                max: 100,
            },
            CombatState::default(),
            SpawnPoint {
                position: Vec3::ZERO,
            },
            Transform::from_translation(Vec3::ZERO),
            ActionState::<AfterglowAction>::default(),
        ),
    );
    let bob = rig.spawn_replicated(
        BOB,
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
            SpawnPoint {
                position: Vec3::new(5.0, 0.0, 0.0),
            },
            Transform::from_translation(Vec3::new(5.0, 0.0, 0.0)),
            ActionState::<AfterglowAction>::default(),
        ),
    );

    rig.queue_action(1, {
        let alice = alice;
        move |app| {
            let mut state = ActionState::<AfterglowAction>::default();
            state.press(&AfterglowAction::AttackPrimary);
            app.world_mut().entity_mut(alice).insert(state);
        }
    });
    rig.queue_action(2, {
        let alice = alice;
        move |app| {
            app.world_mut()
                .entity_mut(alice)
                .insert(ActionState::<AfterglowAction>::default());
        }
    });
    rig.queue_action(3, {
        let alice = alice;
        move |app| {
            let mut state = ActionState::<AfterglowAction>::default();
            state.press(&AfterglowAction::AttackPrimary);
            app.world_mut().entity_mut(alice).insert(state);
        }
    });

    rig.advance_to(10);

    assert_eq!(
        rig.server_component::<ManaPool>(alice).unwrap().current,
        20,
        "ALICE should have 20 mana after one successful attack (50-30)"
    );
    assert_eq!(
        rig.server_component::<Health>(bob).unwrap().current,
        66,
        "BOB should have taken exactly one attack of damage (100-34=66)"
    );
}

#[test]
fn aoe_damage_hits_multiple_targets() {
    let mut rig = LightyearTestRig::new(1, |_| {}, register_rpg);

    let alice = rig.spawn_replicated(
        ALICE,
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
            Transform::from_translation(Vec3::ZERO),
            ActionState::<AfterglowAction>::default(),
        ),
    );

    let bob = rig.spawn_replicated(
        BOB,
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
            Transform::from_translation(Vec3::new(0.0, 0.0, -3.0)),
            ActionState::<AfterglowAction>::default(),
        ),
    );

    let charlie = rig.spawn_replicated(
        CHARLIE,
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
            Transform::from_translation(Vec3::new(5.0, 0.0, 0.0)),
            ActionState::<AfterglowAction>::default(),
        ),
    );

    let dave = rig.spawn_replicated(
        DAVE,
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
            Transform::from_translation(Vec3::new(0.0, 0.0, 8.0)),
            ActionState::<AfterglowAction>::default(),
        ),
    );

    rig.queue_action(1, {
        let alice = alice;
        move |app| {
            let mut state = ActionState::<AfterglowAction>::default();
            state.press(&AfterglowAction::AttackSecondary);
            app.world_mut().entity_mut(alice).insert(state);
        }
    });

    rig.advance(3);

    let bob_hp = rig.server_component::<Health>(bob).unwrap().current;
    let charlie_hp = rig.server_component::<Health>(charlie).unwrap().current;
    let dave_hp = rig.server_component::<Health>(dave).unwrap().current;

    assert!(
        bob_hp < 100,
        "BOB (3 units) should take AoE damage: hp={}",
        bob_hp,
    );
    assert!(
        charlie_hp < 100,
        "CHARLIE (5 units) should take AoE damage: hp={}",
        charlie_hp,
    );
    assert!(
        dave_hp < 100,
        "DAVE (8 units) should take AoE damage: hp={}",
        dave_hp,
    );

    assert_eq!(
        rig.server_component::<Health>(alice).unwrap().current,
        100,
        "ALICE (attacker) should not take self-damage",
    );

    assert_eq!(
        bob_hp, charlie_hp,
        "all enemies within AoE should take equal damage",
    );
    assert_eq!(
        charlie_hp, dave_hp,
        "all enemies within AoE should take equal damage",
    );
}
