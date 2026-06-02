use super::*;

#[test]
fn udp_pvp_1v1_melee_combat() {
    let mut rig = udp_combat_rig(2);
    let t = rig.current_tick();
    let alice = combat_spawn_player(&mut rig, ALICE, Vec3::ZERO, Team(1));
    let bob = combat_spawn_player(&mut rig, BOB, Vec3::new(5.0, 0.0, 0.0), Team(2));

    rig.queue_action(t + 1, {
        let alice = alice;
        move |app| set_action(app.world_mut(), alice, AfterglowAction::AttackPrimary)
    });
    rig.queue_action(t + 1, {
        let bob = bob;
        move |app| set_action(app.world_mut(), bob, AfterglowAction::AttackPrimary)
    });
    rig.advance_to(t + 4);

    assert_eq!(rig.server_component::<Health>(alice).unwrap().current, 66);
    assert_eq!(rig.server_component::<Health>(bob).unwrap().current, 66);
}

#[test]
fn udp_pvp_shield_blocks_all_attack_types() {
    let mut rig = udp_combat_rig(2);
    let t = rig.current_tick();
    let alice = combat_spawn_player(&mut rig, ALICE, Vec3::ZERO, Team(1));
    let bob = combat_spawn_player(&mut rig, BOB, Vec3::new(5.0, 0.0, 0.0), Team(2));

    rig.queue_action(t + 1, {
        let bob = bob;
        move |app| set_action(app.world_mut(), bob, AfterglowAction::RaiseShield)
    });
    rig.queue_action(t + 2, {
        let alice = alice;
        move |app| set_action(app.world_mut(), alice, AfterglowAction::AttackPrimary)
    });
    rig.queue_action(t + 3, {
        let alice = alice;
        move |app| set_action(app.world_mut(), alice, AfterglowAction::AttackSecondary)
    });
    rig.advance_to(t + 7);

    assert_eq!(rig.server_component::<Health>(bob).unwrap().current, 100);
    assert!(!rig.server_component::<CombatState>(bob).unwrap().dead);
}

#[test]
fn udp_pvp_team_no_friendly_fire() {
    let mut rig = udp_combat_rig(2);
    let t = rig.current_tick();
    let alice = combat_spawn_player(&mut rig, ALICE, Vec3::ZERO, Team(1));
    let bob = combat_spawn_player(&mut rig, BOB, Vec3::new(5.0, 0.0, 0.0), Team(1));

    rig.queue_action(t + 1, {
        let alice = alice;
        move |app| set_action(app.world_mut(), alice, AfterglowAction::AttackPrimary)
    });
    rig.advance_to(t + 5);

    assert_eq!(rig.server_component::<Health>(bob).unwrap().current, 100);
}

#[test]
fn udp_pve_player_vs_enemy() {
    let mut rig = udp_combat_rig(1);
    let t = rig.current_tick();
    let _player = combat_spawn_player(&mut rig, ALICE, Vec3::ZERO, Team(1));
    let enemy = spawn_enemy(&mut rig, ENEMY_ID, Vec3::new(5.0, 0.0, 0.0), 50);
    let alice_server = rig.server_entity(ALICE);

    rig.queue_action(t + 1, move |app| {
        set_action(
            app.world_mut(),
            alice_server,
            AfterglowAction::AttackPrimary,
        )
    });
    rig.advance_to(t + 4);

    let enemy_hp = rig.server_app.world().get::<Health>(enemy).unwrap().current;
    assert_eq!(enemy_hp, 16, "Enemy must take damage (50 - 34)");
}
