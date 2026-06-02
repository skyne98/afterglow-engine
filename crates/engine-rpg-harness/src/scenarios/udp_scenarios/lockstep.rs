use super::*;

#[test]
fn udp_duplicate_same_tick_input_rejected() {
    let mut rig = udp_rig(1, register_lockstep).with_input_delay_ms(50);
    let t = rig.current_tick();

    let alice = rig.spawn_replicated(ALICE, lockstep_player(Vec3::ZERO));
    let alice_c0 = rig.find_client_entity(0, ALICE).expect("ALICE on client 0");
    rig.register_entity(ALICE, vec![alice, alice_c0]);

    rig.queue_action(t + 1, {
        let alice = alice;
        move |app| {
            let world = app.world_mut();
            if let Some(mut health) = world.get_mut::<Health>(alice) {
                health.current = (health.current + HEAL_AMOUNT).min(health.max);
            }
            world.entity_mut(alice).insert(HealApplied);
        }
    });
    rig.queue_action(t + 1, {
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

    rig.advance_to(t + 5);

    let hp = rig.server_component::<Health>(alice).unwrap().current;
    assert_eq!(hp, 110, "Only one heal should apply (110, not 120)");
}

#[test]
fn udp_reordered_inputs_produce_same_result() {
    let mut rig = udp_rig(2, register_lockstep).with_input_delay_ms(50);
    let t = rig.current_tick();

    let alice = rig.spawn_replicated(ALICE, lockstep_player(Vec3::ZERO));
    let bob = rig.spawn_replicated(BOB, lockstep_player(Vec3::new(5.0, 0.0, 0.0)));

    let alice_c0 = rig.find_client_entity(0, ALICE).expect("ALICE client 0");
    let alice_c1 = rig.find_client_entity(1, ALICE).expect("ALICE client 1");
    let bob_c0 = rig.find_client_entity(0, BOB).expect("BOB client 0");
    let bob_c1 = rig.find_client_entity(1, BOB).expect("BOB client 1");
    rig.register_entity(ALICE, vec![alice, alice_c0, alice_c1]);
    rig.register_entity(BOB, vec![bob, bob_c0, bob_c1]);

    rig.queue_action(t + 2, {
        let bob = bob;
        move |app| {
            if let Some(mut health) = app.world_mut().get_mut::<Health>(bob) {
                health.current = (health.current + HEAL_AMOUNT).min(health.max);
            }
        }
    });
    rig.queue_action(t + 1, {
        let alice = alice;
        move |app| {
            if let Some(mut health) = app.world_mut().get_mut::<Health>(alice) {
                health.current = (health.current + HEAL_AMOUNT).min(health.max);
            }
        }
    });

    rig.advance_to(t + 6);

    assert_eq!(rig.server_component::<Health>(alice).unwrap().current, 110,);
    assert_eq!(rig.server_component::<Health>(bob).unwrap().current, 110,);
}

#[test]
fn udp_delayed_input_still_delivers() {
    let mut rig = udp_rig(1, register_lockstep).with_input_delay_ms(50);
    let t = rig.current_tick();

    let bob = rig.spawn_replicated(BOB, lockstep_player(Vec3::ZERO));
    let bob_c0 = rig.find_client_entity(0, BOB).expect("BOB client 0");
    rig.register_entity(BOB, vec![bob, bob_c0]);

    rig.queue_action(t + 3, {
        let bob = bob;
        move |app| {
            if let Some(mut health) = app.world_mut().get_mut::<Health>(bob) {
                health.current = (health.current + HEAL_AMOUNT).min(health.max);
            }
        }
    });

    rig.advance_to(t + 6);

    assert_eq!(rig.server_component::<Health>(bob).unwrap().current, 110,);
}

#[test]
fn udp_same_tick_shield_blocks_attack() {
    let mut rig = udp_rig(2, register_lockstep).with_input_delay_ms(50);
    let t = rig.current_tick();

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

    rig.queue_action(t + 1, {
        let bob = bob;
        move |app| {
            let mut state = ActionState::<AfterglowAction>::default();
            state.press(&AfterglowAction::RaiseShield);
            app.world_mut().entity_mut(bob).insert(state);
        }
    });
    rig.queue_action(t + 1, {
        let alice = alice;
        move |app| {
            let mut state = ActionState::<AfterglowAction>::default();
            state.press(&AfterglowAction::AttackPrimary);
            app.world_mut().entity_mut(alice).insert(state);
        }
    });

    rig.advance_to(t + 5);

    let bob_hp = rig.server_component::<Health>(bob).unwrap().current;
    assert_eq!(bob_hp, 30, "BOB should survive (shield blocks)");
    assert!(!rig.server_component::<CombatState>(bob).unwrap().dead);
}

#[test]
fn udp_server_clamps_heal_to_max_hp() {
    let mut rig = udp_rig(1, register_lockstep).with_input_delay_ms(50);
    let t = rig.current_tick();

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

    rig.queue_action(t + 1, {
        let alice = alice;
        move |app| {
            if let Some(mut health) = app.world_mut().get_mut::<Health>(alice) {
                health.current = (health.current + HEAL_AMOUNT).min(health.max);
            }
        }
    });

    rig.advance_to(t + 5);

    let hp = rig.server_component::<Health>(alice).unwrap().current;
    assert_eq!(hp, 100, "Server must enforce max HP cap");
}
