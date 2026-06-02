use crate::rig::LightyearTestRig;
use afterglow_engine::{
    core::identity::StableEntityId,
    input::AfterglowAction,
    network::{HistoryTick, LightyearRole},
};
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::*;

use super::{components::*, systems::*};

const ALICE: StableEntityId = StableEntityId::from_raw(1);
const BOB: StableEntityId = StableEntityId::from_raw(2);

fn player_bundle(pos: Vec3) -> impl Bundle {
    (
        Health {
            current: 100,
            max: 100,
        },
        CombatState::default(),
        Transform::from_translation(pos),
        ActionState::<AfterglowAction>::default(),
    )
}

fn register_adversarial(app: &mut App, _role: LightyearRole) {
    app.init_resource::<HistoryTick>();
    app.register_component::<StableEntityId>();
    app.register_component::<Health>().add_prediction();
    app.register_component::<CombatState>().add_prediction();
    app.add_systems(
        FixedUpdate,
        (advance_history_tick, resolve_attacks, move_players).chain(),
    );
}

fn set_action_state(world: &mut World, entity: Entity, action: AfterglowAction) {
    let mut state = ActionState::<AfterglowAction>::default();
    state.press(&action);
    world.entity_mut(entity).insert(state);
}

#[test]
fn queue_action_defers_until_intended_tick() {
    let mut rig = LightyearTestRig::new(1, |_| {}, register_adversarial).with_input_delay_ms(50);

    let alice = rig.spawn_replicated(ALICE, player_bundle(Vec3::ZERO));
    let alice_c0 = rig.find_client_entity(0, ALICE).expect("ALICE on client 0");
    rig.register_entity(ALICE, vec![alice, alice_c0]);

    rig.queue_action(50, {
        let alice = alice;
        move |app| {
            if let Some(mut health) = app.world_mut().get_mut::<Health>(alice) {
                health.current = 0;
            }
        }
    });

    rig.advance_to(5);

    let hp = rig.server_component::<Health>(alice).unwrap().current;
    assert_eq!(hp, 100, "Future-tick input must not be processed early");
}

#[test]
fn nan_inf_action_values_clamped() {
    let mut rig = LightyearTestRig::new(1, |_| {}, register_adversarial);

    let alice = rig.spawn_replicated(ALICE, player_bundle(Vec3::ZERO));
    let alice_c0 = rig.find_client_entity(0, ALICE).expect("ALICE on client 0");
    rig.register_entity(ALICE, vec![alice, alice_c0]);

    let mut state = ActionState::<AfterglowAction>::default();
    state.set_axis_pair(&AfterglowAction::Move, Vec2::new(f32::NAN, f32::INFINITY));
    rig.server_app.world_mut().entity_mut(alice).insert(state);

    rig.advance_to(5);

    let pos = rig
        .server_component::<Transform>(alice)
        .unwrap()
        .translation;
    assert!(
        pos.is_finite(),
        "Position must remain finite after NaN/INF input: {pos:?}"
    );
    assert!(
        pos.length() < 10.0,
        "Position must stay within reasonable bounds: {pos:?}"
    );
}

#[test]
fn zero_damage_attack_noop() {
    let mut rig = LightyearTestRig::new(2, |_| {}, register_adversarial).with_input_delay_ms(50);

    let alice = rig.spawn_replicated(ALICE, player_bundle(Vec3::ZERO));
    let bob = rig.spawn_replicated(BOB, player_bundle(Vec3::new(20.0, 0.0, 0.0)));

    let alice_c0 = rig.find_client_entity(0, ALICE).expect("ALICE client 0");
    let alice_c1 = rig.find_client_entity(1, ALICE).expect("ALICE client 1");
    let bob_c0 = rig.find_client_entity(0, BOB).expect("BOB client 0");
    let bob_c1 = rig.find_client_entity(1, BOB).expect("BOB client 1");
    rig.register_entity(ALICE, vec![alice, alice_c0, alice_c1]);
    rig.register_entity(BOB, vec![bob, bob_c0, bob_c1]);

    set_action_state(
        rig.client_world_mut(0),
        alice_c0,
        AfterglowAction::AttackPrimary,
    );

    rig.queue_action(1, {
        let alice = alice;
        move |app| {
            set_action_state(app.world_mut(), alice, AfterglowAction::AttackPrimary);
        }
    });

    rig.advance_to(5);

    let hp = rig.server_component::<Health>(bob).unwrap().current;
    assert_eq!(hp, 100, "Out-of-range attack must not change HP");
}
