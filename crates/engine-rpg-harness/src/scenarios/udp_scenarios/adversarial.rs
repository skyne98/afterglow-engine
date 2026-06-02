use super::*;

#[test]
fn udp_queue_action_defers_until_intended_tick() {
    let mut rig = udp_rig(1, register_adversarial).with_input_delay_ms(50);
    let t = rig.current_tick();

    let alice = rig.spawn_replicated(ALICE, lockstep_player(Vec3::ZERO));
    let alice_c0 = rig.find_client_entity(0, ALICE).expect("ALICE on client 0");
    rig.register_entity(ALICE, vec![alice, alice_c0]);

    rig.queue_action(t + 50, {
        let alice = alice;
        move |app| {
            if let Some(mut health) = app.world_mut().get_mut::<Health>(alice) {
                health.current = 0;
            }
        }
    });

    rig.advance_to(t + 5);

    let hp = rig.server_component::<Health>(alice).unwrap().current;
    assert_eq!(hp, 100, "Future-tick input must not be processed early");
}

#[test]
fn udp_nan_inf_action_values_clamped() {
    let mut rig = udp_rig(1, register_adversarial);
    let t = rig.current_tick();

    let alice = rig.spawn_replicated(ALICE, lockstep_player(Vec3::ZERO));
    let alice_c0 = rig.find_client_entity(0, ALICE).expect("ALICE on client 0");
    rig.register_entity(ALICE, vec![alice, alice_c0]);

    let mut state = ActionState::<AfterglowAction>::default();
    state.set_axis_pair(&AfterglowAction::Move, Vec2::new(f32::NAN, f32::INFINITY));
    rig.server_app.world_mut().entity_mut(alice).insert(state);

    rig.advance_to(t + 5);

    let pos = rig
        .server_component::<Transform>(alice)
        .unwrap()
        .translation;
    assert!(pos.is_finite(), "Position must remain finite: {pos:?}");
    assert!(pos.length() < 10.0, "Position out of bounds: {pos:?}");
}

#[test]
fn udp_zero_damage_attack_noop() {
    let mut rig = udp_rig(2, register_adversarial).with_input_delay_ms(50);
    let t = rig.current_tick();

    let alice = rig.spawn_replicated(ALICE, lockstep_player(Vec3::ZERO));
    let bob = rig.spawn_replicated(BOB, lockstep_player(Vec3::new(20.0, 0.0, 0.0)));

    let alice_c0 = rig.find_client_entity(0, ALICE).expect("ALICE client 0");
    let alice_c1 = rig.find_client_entity(1, ALICE).expect("ALICE client 1");
    let bob_c0 = rig.find_client_entity(0, BOB).expect("BOB client 0");
    let bob_c1 = rig.find_client_entity(1, BOB).expect("BOB client 1");
    rig.register_entity(ALICE, vec![alice, alice_c0, alice_c1]);
    rig.register_entity(BOB, vec![bob, bob_c0, bob_c1]);

    rig.queue_action(t + 1, {
        let alice = alice;
        move |app| {
            set_action(app.world_mut(), alice, AfterglowAction::AttackPrimary);
        }
    });

    rig.advance_to(t + 5);

    let hp = rig.server_component::<Health>(bob).unwrap().current;
    assert_eq!(hp, 100, "Out-of-range attack must not change HP");
}
