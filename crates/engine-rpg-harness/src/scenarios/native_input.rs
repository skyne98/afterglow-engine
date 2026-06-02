//! Crossbeam tests for Lightyear's Leafwing input plugin path.
//! Shares test bodies with the UDP variants in udp_scenarios/native_input.rs.

use super::udp_scenarios::native_input::{
    assert_native_input_link_ready, register_native_input, set_desired_input,
    set_fixed_native_input_delay, setup_client_native_input, spawn_player,
    wait_for_native_input_sync,
};
use crate::rig::LightyearTestRig;
use afterglow_engine::{core::identity::StableEntityId, input::AfterglowAction};
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;

const ALICE: StableEntityId = StableEntityId::from_raw(1);

fn native_input_rig_crossbeam(client_count: usize) -> LightyearTestRig {
    let mut rig = LightyearTestRig::new(client_count, |_| {}, register_native_input);
    set_fixed_native_input_delay(&mut rig, 2);
    wait_for_native_input_sync(&mut rig);
    rig
}

#[test]
fn native_input_local_client_state() {
    let mut rig = native_input_rig_crossbeam(1);

    spawn_player(&mut rig, ALICE, Vec3::ZERO);
    setup_client_native_input(&mut rig, 0, ALICE);
    assert_native_input_link_ready(&rig, 0);

    let entity = rig.client_entity(ALICE, 0);
    let mut state = ActionState::<AfterglowAction>::default();
    state.set_axis_pair(&AfterglowAction::Move, Vec2::new(0.0, 1.0));
    set_desired_input(&mut rig, 0, state);

    rig.advance(5);

    let client_state = rig
        .client_component::<ActionState<AfterglowAction>>(0, entity)
        .unwrap();
    assert!(
        client_state.axis_pair(&AfterglowAction::Move).y > 0.0,
        "apply_desired_input should set client ActionState via WriteClientInputs"
    );
}

#[test]
fn native_input_infrastructure_setup() {
    let mut rig = native_input_rig_crossbeam(1);
    spawn_player(&mut rig, ALICE, Vec3::ZERO);
    setup_client_native_input(&mut rig, 0, ALICE);
    assert_native_input_link_ready(&rig, 0);
}
