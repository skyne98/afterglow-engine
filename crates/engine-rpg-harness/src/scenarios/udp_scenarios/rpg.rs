use super::*;

#[test]
fn udp_networked_movement() {
    let mut rig = udp_rig(1, register_rpg);
    let t = rig.current_tick();

    let alice = rig.spawn_replicated(ALICE, rpg_player(Vec3::ZERO));
    let alice_c0 = rig.find_client_entity(0, ALICE).expect("ALICE on client 0");
    rig.register_entity(ALICE, vec![alice, alice_c0]);

    let mut input_state = ActionState::<AfterglowAction>::default();
    input_state.set_axis_pair(&AfterglowAction::Move, Vec2::new(0.0, 1.0));
    rig.client_world_mut(0)
        .entity_mut(alice_c0)
        .insert(input_state);

    rig.queue_action(t + 1, {
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

    rig.advance(20);

    let server_pos_after = rig
        .server_component::<Transform>(alice)
        .unwrap()
        .translation;
    let moved = server_pos_after.distance(server_pos_before);
    assert!(
        moved > 0.2,
        "Player should move after advancing 20 ticks: moved={moved}"
    );

    let client_pos = rig
        .client_component::<Transform>(0, alice_c0)
        .unwrap()
        .translation;
    let diff = (server_pos_after - client_pos).length();
    assert!(
        diff < 1.0,
        "UDP: Client position should roughly match server after replication: \
         server={server_pos_after:.3} client={client_pos:.3} diff={diff:.3}"
    );
}

#[test]
fn udp_death_respawn_cycle() {
    let mut rig = udp_rig(1, register_rpg);
    let t = rig.current_tick();

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

    rig.queue_action(t + 1, {
        let bob = bob;
        move |app| {
            let mut state = ActionState::<AfterglowAction>::default();
            state.press(&AfterglowAction::AttackPrimary);
            app.world_mut().entity_mut(bob).insert(state);
        }
    });
    rig.queue_action(t + 2, {
        let bob = bob;
        move |app| clear_action(app.world_mut(), bob)
    });

    rig.advance_to(t + 5);

    assert!(rig.server_component::<CombatState>(alice).unwrap().dead);
    assert_eq!(rig.server_component::<Health>(alice).unwrap().current, 0);

    rig.advance_to(t + 30);

    assert!(!rig.server_component::<CombatState>(alice).unwrap().dead);
    assert_eq!(rig.server_component::<Health>(alice).unwrap().current, 100);
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
fn udp_status_effects_over_time() {
    let mut rig = udp_rig(1, register_rpg);

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
    assert_eq!(rig.server_component::<Health>(alice).unwrap().current, 75);

    rig.advance(5);
    assert_eq!(rig.server_component::<Health>(alice).unwrap().current, 50);
    assert!(rig.server_component::<BurnEffect>(alice).is_none());
}

#[test]
fn udp_cooldown_and_resource_cost() {
    let mut rig = udp_rig(2, register_rpg);
    let t = rig.current_tick();

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

    rig.queue_action(t + 1, {
        let alice = alice;
        move |app| {
            let mut state = ActionState::<AfterglowAction>::default();
            state.press(&AfterglowAction::AttackPrimary);
            app.world_mut().entity_mut(alice).insert(state);
        }
    });
    rig.queue_action(t + 2, {
        let alice = alice;
        move |app| clear_action(app.world_mut(), alice)
    });
    rig.queue_action(t + 3, {
        let alice = alice;
        move |app| {
            let mut state = ActionState::<AfterglowAction>::default();
            state.press(&AfterglowAction::AttackPrimary);
            app.world_mut().entity_mut(alice).insert(state);
        }
    });
    rig.queue_action(t + 4, {
        let alice = alice;
        move |app| clear_action(app.world_mut(), alice)
    });

    rig.advance_to(t + 10);

    assert_eq!(rig.server_component::<ManaPool>(alice).unwrap().current, 20);
    assert_eq!(
        rig.server_component::<Health>(bob).unwrap().current,
        66,
        "Second attack within cooldown window must be blocked"
    );
}
