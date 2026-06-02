use super::*;

#[test]
fn pve_player_vs_enemy() {
    let mut rig = setup_multiplayer_rig(1, true);
    let _player = spawn_player(&mut rig, ALICE, Vec3::ZERO, Team(1));
    let enemy = spawn_enemy(&mut rig, ENEMY_ID, Vec3::new(5.0, 0.0, 0.0), 50, None);
    let alice_server = rig.server_entity(ALICE);

    rig.queue_action(1, move |app| {
        set_action_state(
            app.world_mut(),
            alice_server,
            AfterglowAction::AttackPrimary,
        )
    });
    rig.advance_to(DELIVERY_TICK);

    let enemy_hp = rig.server_app.world().get::<Health>(enemy).unwrap().current;
    assert_eq!(enemy_hp, 16, "Enemy must take damage (50 - 34)");
}

#[test]
fn pve_enemy_respawns() {
    let mut rig = setup_multiplayer_rig(1, true);
    let _player = spawn_player(&mut rig, ALICE, Vec3::ZERO, Team(1));
    let spawn_pos = Vec3::new(5.0, 0.0, 0.0);
    let enemy = spawn_enemy(&mut rig, ENEMY_ID, spawn_pos, 20, Some(spawn_pos));
    let alice_server = rig.server_entity(ALICE);

    // Attack once to kill enemy.
    rig.queue_action(1, move |app| {
        set_action_state(
            app.world_mut(),
            alice_server,
            AfterglowAction::AttackPrimary,
        )
    });
    // Clear action at tick 2 (delivers ~tick 5) so ALICE stops attacking.
    rig.queue_action(2, {
        let alice_server = alice_server;
        move |app| clear_action_state(app.world_mut(), alice_server)
    });

    // Advance to tick 4: enemy dies (20 -> 0 HP).
    // DeadTimer starts at tick 4, counts down 10 ticks -> respawn at tick 15.
    rig.advance_to(20);

    let enemy_health = rig.server_app.world().get::<Health>(enemy).unwrap();
    let enemy_combat = rig.server_app.world().get::<CombatState>(enemy).unwrap();
    assert!(!enemy_combat.dead, "Enemy must have respawned (alive)");
    assert_eq!(
        enemy_health.current, enemy_health.max,
        "Enemy must respawn with full HP"
    );
}

#[test]
fn pve_boss_multiple_phases() {
    let mut rig = setup_multiplayer_rig(1, true);
    let _player = spawn_player(&mut rig, ALICE, Vec3::ZERO, Team(1));
    // Boss at distance 2 so 3 knockbacks (2.0 each) still leave it in range.
    let boss = spawn_boss(
        &mut rig,
        BOSS_ID,
        Vec3::new(2.0, 0.0, 0.0),
        100,
        vec![70, 30],
    );
    let alice_server = rig.server_entity(ALICE);

    // Phase 1 -> Phase 2: Attack once -> 100 -> 66 HP <= 70 -> phase 2.
    rig.queue_action(1, move |app| {
        set_action_state(
            app.world_mut(),
            alice_server,
            AfterglowAction::AttackPrimary,
        )
    });
    rig.queue_action(2, {
        let alice_server = alice_server;
        move |app| clear_action_state(app.world_mut(), alice_server)
    });
    rig.advance_to(DELIVERY_TICK);

    let boss_hp1 = rig.server_app.world().get::<Health>(boss).unwrap();
    let boss_p1 = rig.server_app.world().get::<Boss>(boss).unwrap();
    assert_eq!(
        boss_hp1.current, 66,
        "Boss should be at 66 HP after one hit"
    );
    assert!(
        boss_p1.phase >= 2,
        "Boss must reach phase 2 at <=70 HP (phase={})",
        boss_p1.phase
    );

    // Phase 2 stays: second attack -> 66 -> 32 HP (32 > 30, still phase 2).
    rig.queue_action(10, {
        let alice_server = alice_server;
        move |app| {
            set_action_state(
                app.world_mut(),
                alice_server,
                AfterglowAction::AttackPrimary,
            )
        }
    });
    rig.queue_action(11, {
        let alice_server = alice_server;
        move |app| clear_action_state(app.world_mut(), alice_server)
    });
    rig.advance_to(14);

    let boss_hp2 = rig.server_app.world().get::<Health>(boss).unwrap();
    assert_eq!(
        boss_hp2.current, 32,
        "Boss should be at 32 HP after two hits"
    );

    // Phase 3: third attack -> 32 -> -2 -> 0 <= 30 -> phase 3.
    rig.queue_action(15, {
        let alice_server = alice_server;
        move |app| {
            set_action_state(
                app.world_mut(),
                alice_server,
                AfterglowAction::AttackPrimary,
            )
        }
    });
    rig.queue_action(16, {
        let alice_server = alice_server;
        move |app| clear_action_state(app.world_mut(), alice_server)
    });
    rig.advance_to(20);

    let boss_hp3 = rig.server_app.world().get::<Health>(boss).unwrap();
    let boss_p3 = rig.server_app.world().get::<Boss>(boss).unwrap();
    assert_eq!(
        boss_hp3.current, 0,
        "Boss must die by tick 20 (HP={})",
        boss_hp3.current
    );
    assert!(
        boss_p3.phase >= 3,
        "Boss must reach phase >=3 (phase={})",
        boss_p3.phase
    );
}
