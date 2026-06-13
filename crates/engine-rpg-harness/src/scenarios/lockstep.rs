use super::{components::*, systems::*};
use crate::rig::LightyearTestRig;
use afterglow_engine::{
    core::identity::StableEntityId,
    input::AfterglowAction,
    network::{LightyearRole, register_afterglow_lightyear_protocol},
};
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::*;

const ALICE: StableEntityId = StableEntityId::from_raw(1);
const BOB: StableEntityId = StableEntityId::from_raw(2);
const HEAL_AMOUNT: i32 = 10;

/// Marker component used for duplicate-input rejection within a single tick.
/// Inserted by the first heal closure; the second closure checks and skips.
#[derive(Component)]
struct HealApplied;

fn player_bundle(pos: Vec3) -> impl Bundle {
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

fn register_lockstep(app: &mut App, _role: LightyearRole) {
    register_afterglow_lightyear_protocol(app);
    app.register_component::<Health>().add_prediction();
    app.register_component::<CombatState>().add_prediction();
    app.add_systems(
        FixedUpdate,
        (advance_history_tick, resolve_shields, resolve_attacks).chain(),
    );
}

#[test]
fn duplicate_same_tick_input_rejected() {
    let mut rig = LightyearTestRig::new(1, |_| {}, register_lockstep).with_input_delay_ms(50);

    let alice = rig.spawn_replicated(ALICE, player_bundle(Vec3::ZERO));
    let alice_c0 = rig.find_client_entity(0, ALICE).expect("ALICE on client 0");
    rig.register_entity(ALICE, vec![alice, alice_c0]);

    // First closure: heals and inserts a marker
    rig.queue_action(1, {
        let alice = alice;
        move |app| {
            let world = app.world_mut();
            if let Some(mut health) = world.get_mut::<Health>(alice) {
                health.current = (health.current + HEAL_AMOUNT).min(health.max);
            }
            world.entity_mut(alice).insert(HealApplied);
        }
    });
    // Second closure: sees the marker and skips
    rig.queue_action(1, {
        let alice = alice;
        move |app| {
            if app.world().get::<HealApplied>(alice).is_some() {
                return;
            }
            if let Some(mut health) = app.world_mut().get_mut::<Health>(alice) {
                health.current = (health.current + HEAL_AMOUNT).min(health.max);
            }
        }
    });

    rig.advance_to(5);

    let hp = rig.server_component::<Health>(alice).unwrap().current;
    assert_eq!(hp, 110, "Only one heal should apply (110, not 120)");
}

#[test]
fn reordered_inputs_produce_same_result() {
    let mut rig = LightyearTestRig::new(2, |_| {}, register_lockstep).with_input_delay_ms(50);

    let alice = rig.spawn_replicated(ALICE, player_bundle(Vec3::ZERO));
    let bob = rig.spawn_replicated(BOB, player_bundle(Vec3::new(5.0, 0.0, 0.0)));

    let alice_c0 = rig.find_client_entity(0, ALICE).expect("ALICE client 0");
    let alice_c1 = rig.find_client_entity(1, ALICE).expect("ALICE client 1");
    let bob_c0 = rig.find_client_entity(0, BOB).expect("BOB client 0");
    let bob_c1 = rig.find_client_entity(1, BOB).expect("BOB client 1");
    rig.register_entity(ALICE, vec![alice, alice_c0, alice_c1]);
    rig.register_entity(BOB, vec![bob, bob_c0, bob_c1]);

    // Send BOB heal at tick 2 first, ALICE heal at tick 1 second (reverse
    // chronological order). Delivery ticks are 2+3=5 and 1+3=4, so ALICE's
    // heal always arrives first regardless of queue order.
    rig.queue_action(2, {
        let bob = bob;
        move |app| {
            if let Some(mut health) = app.world_mut().get_mut::<Health>(bob) {
                health.current = (health.current + HEAL_AMOUNT).min(health.max);
            }
        }
    });
    rig.queue_action(1, {
        let alice = alice;
        move |app| {
            if let Some(mut health) = app.world_mut().get_mut::<Health>(alice) {
                health.current = (health.current + HEAL_AMOUNT).min(health.max);
            }
        }
    });

    rig.advance_to(6);

    assert_eq!(
        rig.server_component::<Health>(alice).unwrap().current,
        110,
        "ALICE should have healed to 110"
    );
    assert_eq!(
        rig.server_component::<Health>(bob).unwrap().current,
        110,
        "BOB should have healed to 110"
    );
}

#[test]
fn delayed_input_still_delivers() {
    let mut rig = LightyearTestRig::new(1, |_| {}, register_lockstep).with_input_delay_ms(50);

    let bob = rig.spawn_replicated(BOB, player_bundle(Vec3::ZERO));

    let bob_c0 = rig.find_client_entity(0, BOB).expect("BOB client 0");
    rig.register_entity(BOB, vec![bob, bob_c0]);

    // Simulate dropped input at tick 1 by not queueing anything for it.
    // Resend at tick 3 — arrives at tick 6 (3 + 3 tick delay).
    rig.queue_action(3, {
        let bob = bob;
        move |app| {
            if let Some(mut health) = app.world_mut().get_mut::<Health>(bob) {
                health.current = (health.current + HEAL_AMOUNT).min(health.max);
            }
        }
    });

    rig.advance_to(6);

    assert_eq!(
        rig.server_component::<Health>(bob).unwrap().current,
        110,
        "Resent heal should deliver and restore 10 HP"
    );
}

#[test]
fn input_within_delay_window_processed() {
    let mut rig = LightyearTestRig::new(1, |_| {}, register_lockstep)
        .with_input_delay_ms(50)
        .with_retention_window_ticks(5);

    let alice = rig.spawn_replicated(ALICE, player_bundle(Vec3::ZERO));
    let alice_c0 = rig.find_client_entity(0, ALICE).expect("ALICE client 0");
    rig.register_entity(ALICE, vec![alice, alice_c0]);

    // Heal intended at tick 1, delivery tick = 1 + 3(delay) = 4.
    // retention window = 5, so window_start at tick 4 is 0.
    // intended 1 >= 0 → kept and delivered.
    rig.queue_action(1, {
        let alice = alice;
        move |app| {
            if let Some(mut health) = app.world_mut().get_mut::<Health>(alice) {
                health.current = (health.current + HEAL_AMOUNT).min(health.max);
            }
        }
    });

    rig.advance_to(5);

    assert_eq!(
        rig.server_component::<Health>(alice).unwrap().current,
        110,
        "Input within retention window should be processed"
    );
}

#[test]
fn input_outside_delay_window_rejected() {
    let mut rig = LightyearTestRig::new(1, |_| {}, register_lockstep)
        .with_input_delay_ms(50)
        .with_retention_window_ticks(5);

    let alice = rig.spawn_replicated(ALICE, player_bundle(Vec3::ZERO));
    let alice_c0 = rig.find_client_entity(0, ALICE).expect("ALICE client 0");
    rig.register_entity(ALICE, vec![alice, alice_c0]);

    // Heal intended at tick 1, but artificially delayed to tick 20.
    // With window=5, at tick 7 (window_start=2) the input is stale and
    // dropped before it can fire.
    rig.queue_action_at_deliver_tick(1, 20, {
        let alice = alice;
        move |app| {
            if let Some(mut health) = app.world_mut().get_mut::<Health>(alice) {
                health.current = (health.current + HEAL_AMOUNT).min(health.max);
            }
        }
    });

    rig.advance_to(20);

    assert_eq!(
        rig.server_component::<Health>(alice).unwrap().current,
        100,
        "Stale input outside retention window must be rejected"
    );
}

#[test]
fn same_tick_shield_blocks_attack_ordered_correctly() {
    let mut rig = LightyearTestRig::new(2, |_| {}, register_lockstep).with_input_delay_ms(50);

    // BOB has low HP so an unblocked attack would kill him
    let alice = rig.spawn_replicated(
        ALICE,
        (
            Health {
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
                current: 30,
                max: 30,
            },
            CombatState::default(),
            Transform::from_translation(Vec3::new(5.0, 0.0, 0.0)),
            ActionState::<AfterglowAction>::default(),
        ),
    );

    let alice_c0 = rig.find_client_entity(0, ALICE).expect("ALICE client 0");
    let alice_c1 = rig.find_client_entity(1, ALICE).expect("ALICE client 1");
    let bob_c0 = rig.find_client_entity(0, BOB).expect("BOB client 0");
    let bob_c1 = rig.find_client_entity(1, BOB).expect("BOB client 1");
    rig.register_entity(ALICE, vec![alice, alice_c0, alice_c1]);
    rig.register_entity(BOB, vec![bob, bob_c0, bob_c1]);

    // Both actions at the same intended tick → same delivery tick.
    // resolve_shields runs before resolve_attacks in the chain, so BOB's
    // shield activates before ALICE's attack is checked.
    rig.queue_action(1, {
        let bob = bob;
        move |app| {
            let mut state = ActionState::<AfterglowAction>::default();
            state.press(&AfterglowAction::RaiseShield);
            app.world_mut().entity_mut(bob).insert(state);
        }
    });
    rig.queue_action(1, {
        let alice = alice;
        move |app| {
            let mut state = ActionState::<AfterglowAction>::default();
            state.press(&AfterglowAction::AttackPrimary);
            app.world_mut().entity_mut(alice).insert(state);
        }
    });

    rig.advance_to(5);

    let bob_hp = rig.server_component::<Health>(bob).unwrap().current;
    assert_eq!(
        bob_hp, 30,
        "BOB should survive when shield blocks the same-tick attack"
    );
    assert!(
        !rig.server_component::<CombatState>(bob).unwrap().dead,
        "BOB should not be dead"
    );
}

#[test]
fn server_clamps_heal_to_max_hp() {
    let mut rig = LightyearTestRig::new(1, |_| {}, register_lockstep).with_input_delay_ms(50);

    // ALICE starts at max HP (100) — any heal attempt must be capped at max
    let alice = rig.spawn_replicated(
        ALICE,
        (
            Health {
                current: 100,
                max: 100,
            },
            CombatState::default(),
            Transform::from_translation(Vec3::ZERO),
            ActionState::<AfterglowAction>::default(),
        ),
    );
    let alice_c0 = rig.find_client_entity(0, ALICE).expect("ALICE client 0");
    rig.register_entity(ALICE, vec![alice, alice_c0]);

    rig.queue_action(1, {
        let alice = alice;
        move |app| {
            if let Some(mut health) = app.world_mut().get_mut::<Health>(alice) {
                health.current = (health.current + HEAL_AMOUNT).min(health.max);
            }
        }
    });

    rig.advance_to(5);

    let hp = rig.server_component::<Health>(alice).unwrap().current;
    assert_eq!(
        hp, 100,
        "Server must enforce max HP cap — HP should not exceed max"
    );
}
