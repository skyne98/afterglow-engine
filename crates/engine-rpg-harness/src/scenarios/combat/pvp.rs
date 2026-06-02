use super::*;

#[test]
fn pvp_1v1_melee_combat() {
    let mut rig = setup_multiplayer_rig(2, true);
    let alice = spawn_player(&mut rig, ALICE, Vec3::ZERO, Team(1));
    let bob = spawn_player(&mut rig, BOB, Vec3::new(5.0, 0.0, 0.0), Team(2));

    rig.queue_action(1, {
        let alice = alice;
        move |app| set_action_state(app.world_mut(), alice, AfterglowAction::AttackPrimary)
    });
    rig.queue_action(1, {
        let bob = bob;
        move |app| set_action_state(app.world_mut(), bob, AfterglowAction::AttackPrimary)
    });
    rig.advance_to(DELIVERY_TICK);

    let alice_hp = rig.server_component::<Health>(alice).unwrap().current;
    let bob_hp = rig.server_component::<Health>(bob).unwrap().current;
    assert_eq!(alice_hp, 66, "ALICE should be at 66 HP (100 - 34)");
    assert_eq!(bob_hp, 66, "BOB should be at 66 HP (100 - 34)");
}

#[test]
fn pvp_shield_blocks_all_attack_types() {
    let mut rig = setup_multiplayer_rig(2, true);
    let alice = spawn_player(&mut rig, ALICE, Vec3::ZERO, Team(1));
    let bob = spawn_player(&mut rig, BOB, Vec3::new(5.0, 0.0, 0.0), Team(2));

    rig.queue_action(1, {
        let bob = bob;
        move |app| set_action_state(app.world_mut(), bob, AfterglowAction::RaiseShield)
    });
    rig.queue_action(2, {
        let alice = alice;
        move |app| set_action_state(app.world_mut(), alice, AfterglowAction::AttackPrimary)
    });
    rig.queue_action(3, {
        let alice = alice;
        move |app| set_action_state(app.world_mut(), alice, AfterglowAction::AttackSecondary)
    });
    // Deliveries: shield at 4, primary at 5, secondary at 6.
    // Shield lasts 20 ticks; check at tick 7 that both attacks were blocked.
    rig.advance_to(7);

    let bob_hp = rig.server_component::<Health>(bob).unwrap().current;
    assert_eq!(bob_hp, 100, "BOB shield must block both attack types");
    let bob_combat = rig.server_component::<CombatState>(bob).unwrap();
    assert!(!bob_combat.dead, "BOB must not be dead");
}

#[test]
fn pvp_simultaneous_attacks_on_same_target() {
    let mut rig = setup_multiplayer_rig(3, true);
    // Position both attackers such that BOB is the nearest target for each.
    let alice = spawn_player(&mut rig, ALICE, Vec3::new(-5.0, 0.0, 0.0), Team(1));
    let bob = spawn_player(&mut rig, BOB, Vec3::ZERO, Team(2));
    let carol = spawn_player(&mut rig, CAROL, Vec3::new(5.0, 0.0, 0.0), Team(3));

    rig.queue_action(1, {
        let alice = alice;
        move |app| set_action_state(app.world_mut(), alice, AfterglowAction::AttackPrimary)
    });
    rig.queue_action(1, {
        let carol = carol;
        move |app| set_action_state(app.world_mut(), carol, AfterglowAction::AttackPrimary)
    });
    rig.advance_to(DELIVERY_TICK);

    let bob_hp = rig.server_component::<Health>(bob).unwrap().current;
    assert_eq!(
        bob_hp, 32,
        "BOB must take damage from both attackers (100 - 34 - 34 = 32)"
    );
    let alice_hp = rig.server_component::<Health>(alice).unwrap().current;
    let carol_hp = rig.server_component::<Health>(carol).unwrap().current;
    assert_eq!(alice_hp, 100, "ALICE must not be targeted");
    assert_eq!(carol_hp, 100, "CAROL must not be targeted");
}

#[test]
fn pvp_knockback_impulse() {
    let mut rig = setup_multiplayer_rig(2, true);
    let alice = spawn_player(&mut rig, ALICE, Vec3::ZERO, Team(1));
    let bob = spawn_player(&mut rig, BOB, Vec3::new(5.0, 0.0, 0.0), Team(2));

    let bob_start = rig.server_component::<Transform>(bob).unwrap().translation;

    rig.queue_action(1, {
        let alice = alice;
        move |app| set_action_state(app.world_mut(), alice, AfterglowAction::AttackPrimary)
    });
    rig.advance_to(DELIVERY_TICK);

    let bob_end = rig.server_component::<Transform>(bob).unwrap().translation;
    let diff = bob_end - bob_start;
    let expected_dir = (bob_start - Vec3::ZERO).normalize_or_zero();
    assert!(
        diff.dot(expected_dir) > 0.0,
        "BOB must be pushed away from ALICE: diff {diff:?}, dir {expected_dir:?}"
    );
}

#[test]
fn pvp_aoe_hits_multiple_players() {
    let mut rig = setup_multiplayer_rig(3, true);
    let alice = spawn_player(&mut rig, ALICE, Vec3::ZERO, Team(1));
    let bob = spawn_player(&mut rig, BOB, Vec3::new(5.0, 0.0, 0.0), Team(2));
    let carol = spawn_player(&mut rig, CAROL, Vec3::new(0.0, 0.0, 5.0), Team(3));

    rig.queue_action(1, {
        let alice = alice;
        move |app| set_action_state(app.world_mut(), alice, AfterglowAction::AttackSecondary)
    });
    rig.advance_to(DELIVERY_TICK);

    let bob_hp = rig.server_component::<Health>(bob).unwrap().current;
    let carol_hp = rig.server_component::<Health>(carol).unwrap().current;
    assert_eq!(bob_hp, 75, "BOB must take AOE damage (100 - 25)");
    assert_eq!(carol_hp, 75, "CAROL must take AOE damage (100 - 25)");
}

#[test]
fn pvp_cooldown_prevents_double_attack() {
    let mut rig = setup_multiplayer_rig(2, true);
    *rig.server_app.world_mut().resource_mut::<AttackCooldown>() =
        AttackCooldown(ATTACK_COOLDOWN_TICKS);

    let alice = spawn_player(&mut rig, ALICE, Vec3::ZERO, Team(1));
    let bob = spawn_player(&mut rig, BOB, Vec3::new(5.0, 0.0, 0.0), Team(2));

    // Attack at tick 1 -> tick 4 delivery. ActionState persists on server.
    rig.queue_action(1, {
        let alice = alice;
        move |app| set_action_state(app.world_mut(), alice, AfterglowAction::AttackPrimary)
    });
    // Advance past the delivery and the next tick where cooldown would block.
    rig.advance_to(DELIVERY_TICK + 1);

    let bob_hp = rig.server_component::<Health>(bob).unwrap().current;
    assert_eq!(bob_hp, 66, "BOB should have taken exactly one hit (66 HP)");
}

#[test]
fn pvp_team_no_friendly_fire() {
    let mut rig = setup_multiplayer_rig(2, true);
    let alice = spawn_player(&mut rig, ALICE, Vec3::ZERO, Team(1));
    let bob = spawn_player(&mut rig, BOB, Vec3::new(5.0, 0.0, 0.0), Team(1));

    rig.queue_action(1, {
        let alice = alice;
        move |app| set_action_state(app.world_mut(), alice, AfterglowAction::AttackPrimary)
    });
    // Team check persists across all ticks, so advance_to(5) is fine.
    rig.advance_to(5);

    let bob_hp = rig.server_component::<Health>(bob).unwrap().current;
    assert_eq!(bob_hp, 100, "BOB must NOT take damage from same-team ALICE");
}

#[test]
fn pvp_death_removes_from_combat() {
    let mut rig = LightyearTestRig::new(
        2,
        |app| {
            app.add_plugins(AfterglowPhysicsPlugin);
            app.add_plugins(AfterglowFirstPersonControllerPlugin);
        },
        register_combat,
    )
    .with_input_delay_ms(50);

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
            Team(1),
            FirstPersonController::new(),
        ),
    );
    let bob = rig.spawn_replicated(
        BOB,
        (
            Health {
                current: 20,
                max: 20,
            },
            CombatState::default(),
            Transform::from_translation(Vec3::new(5.0, 0.0, 0.0)),
            ActionState::<AfterglowAction>::default(),
            Team(2),
            FirstPersonController::new(),
        ),
    );

    let alice_c0 = rig.find_client_entity(0, ALICE).unwrap();
    let alice_c1 = rig.find_client_entity(1, ALICE).unwrap();
    let bob_c0 = rig.find_client_entity(0, BOB).unwrap();
    let bob_c1 = rig.find_client_entity(1, BOB).unwrap();
    rig.register_entity(ALICE, vec![alice, alice_c0, alice_c1]);
    rig.register_entity(BOB, vec![bob, bob_c0, bob_c1]);

    rig.queue_action(1, move |app| {
        set_action_state(app.world_mut(), alice, AfterglowAction::AttackPrimary)
    });

    // Advance through death: attack at tick 4 kills Bob (20 HP -> 0),
    // then clear ALICE's action to prevent further hits hitting corpse.
    rig.queue_action(3, move |app| {
        clear_action_state(app.world_mut(), alice);
    });

    rig.advance_to(12);

    let bob_hp = rig.server_component::<Health>(bob).unwrap().current;
    assert_eq!(bob_hp, 0, "BOB should be dead (0 HP)");
    let bob_combat = rig.server_component::<CombatState>(bob).unwrap();
    assert!(bob_combat.dead, "BOB must remain dead");
}
