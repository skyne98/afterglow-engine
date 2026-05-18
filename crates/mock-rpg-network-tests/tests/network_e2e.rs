use afterglow_engine::{core::identity::StableEntityId, network::LightyearRole};
use mock_rpg_network_tests::{Vec3i, network_e2e::*};

#[test]
fn late_shield_rolls_back_death_pickup_and_inventory_through_lightyear() {
    let mut rpg = LightyearNetworkedRpg::new(16);
    let bob_corpse = StableEntityId::from_raw(25_002);
    let bob_loot = StableEntityId::from_raw(30_002);
    assert!(rpg.has_afterglow_network_resources());
    assert!(rpg.has_lightyear_links());
    assert_eq!(
        rpg.client_afterglow_lightyear_role(),
        Some(LightyearRole::Client)
    );
    assert_eq!(
        rpg.server_afterglow_lightyear_role(),
        Some(LightyearRole::Server)
    );
    rpg.send(attack(1, 1, ALICE, BOB, 120), 0);
    rpg.send(pick_up_food(2, 3, ALICE, bob_loot), 0);
    rpg.send(raise_shield(1, 1, BOB), 4);

    rpg.advance_to(4);
    assert_eq!(rpg.hp(BOB), 0);
    assert_eq!(rpg.death_markers_for(BOB), 1);
    assert_eq!(rpg.corpses_for(BOB), 1);
    assert_eq!(rpg.loot_for(BOB), 1);
    assert_eq!(rpg.inventory_food(ALICE), 1);
    assert_eq!(
        rpg.client_confirmed_combatant(BOB),
        Some(Combatant::new(0, Vec3i::new(4, 0, 0)))
    );
    assert!(rpg.client_predicted_combatant(BOB).is_some());
    assert!(rpg.client_confirmed_inventory_food(ALICE).is_some());
    assert!(rpg.client_predicted_inventory_food(ALICE).is_some());
    assert!(rpg.client_has_replicated(bob_corpse));
    assert!(rpg.client_has_replicated(bob_loot));

    rpg.advance_to(5);

    assert_eq!(rpg.received_lightyear_inputs(), 3);
    assert_eq!(rpg.hp(BOB), 100);
    assert_eq!(rpg.death_markers_for(BOB), 0);
    assert_eq!(rpg.corpses_for(BOB), 0);
    assert_eq!(rpg.loot_for(BOB), 0);
    assert_eq!(rpg.inventory_food(ALICE), 0);
    assert!(rpg.client_predicted_combatant(BOB).is_some());
    assert_eq!(
        rpg.client_confirmed_combatant(BOB),
        Some(Combatant {
            hp: 100,
            shield_through: 2,
            position: Vec3i::new(4, 0, 0),
        })
    );
    assert_eq!(rpg.client_predicted_inventory_food(ALICE), Some(0));
    assert_eq!(rpg.client_confirmed_inventory_food(ALICE), Some(0));
    assert!(!rpg.client_has_replicated(bob_corpse));
    assert!(!rpg.client_has_replicated(bob_loot));
    assert!(rpg.client_prediction_history_len::<Combatant>(BOB) > 0);
    assert!(rpg.facts().contains(&CombatFact::SpellBlocked {
        tick: 2,
        target: BOB,
    }));
    assert!(!rpg.facts().contains(&CombatFact::FoodPickedUp {
        tick: 3,
        player: ALICE,
        from: bob_loot,
    }));
    assert!(
        rpg.corrections()
            .contains(&Correction::DespawnEntity(bob_corpse))
    );
    assert!(
        rpg.corrections()
            .contains(&Correction::DespawnEntity(bob_loot))
    );
    assert!(rpg.corrections().contains(&Correction::ComponentChanged {
        stable_id: ALICE,
        component: "Inventory",
    }));
    assert!(rpg.history_len::<Inventory>(ALICE) > 0);
}

#[test]
fn late_shield_input_rolls_back_death_loot_pickup_and_inventory() {
    let mut rpg = NetworkedRpg::new(16);
    let bob_corpse = StableEntityId::from_raw(25_002);
    let bob_loot = StableEntityId::from_raw(30_002);
    assert!(rpg.has_afterglow_network_resources());
    rpg.send(attack(1, 1, ALICE, BOB, 120), 0);
    rpg.send(pick_up_food(2, 3, ALICE, bob_loot), 0);
    rpg.send(raise_shield(1, 1, BOB), 4);

    rpg.advance_to(4);
    assert_eq!(rpg.hp(BOB), 0);
    assert_eq!(rpg.death_markers_for(BOB), 1);
    assert_eq!(rpg.corpses_for(BOB), 1);
    assert_eq!(rpg.loot_for(BOB), 1);
    assert_eq!(rpg.inventory_food(ALICE), 1);
    assert!(rpg.facts().contains(&CombatFact::FoodPickedUp {
        tick: 3,
        player: ALICE,
        from: bob_loot,
    }));

    rpg.advance_to(5);

    assert_eq!(rpg.hp(BOB), 100);
    assert_eq!(rpg.death_markers_for(BOB), 0);
    assert_eq!(rpg.corpses_for(BOB), 0);
    assert_eq!(rpg.loot_for(BOB), 0);
    assert_eq!(rpg.inventory_food(ALICE), 0);
    assert!(rpg.facts().contains(&CombatFact::SpellBlocked {
        tick: 2,
        target: BOB,
    }));
    assert!(!rpg.facts().contains(&CombatFact::PlayerDied {
        tick: 2,
        player: BOB,
    }));
    assert!(!rpg.facts().contains(&CombatFact::FoodPickedUp {
        tick: 3,
        player: ALICE,
        from: bob_loot,
    }));
    assert!(rpg.corrections().contains(&Correction::ComponentChanged {
        stable_id: BOB,
        component: "Combatant",
    }));
    assert!(
        rpg.corrections()
            .contains(&Correction::DespawnEntity(StableEntityId::from_raw(20_002)))
    );
    assert!(
        rpg.corrections()
            .contains(&Correction::DespawnEntity(bob_corpse))
    );
    assert!(
        rpg.corrections()
            .contains(&Correction::DespawnEntity(bob_loot))
    );
    assert!(rpg.corrections().contains(&Correction::ComponentChanged {
        stable_id: ALICE,
        component: "Inventory",
    }));
    assert!(
        rpg.corrections()
            .contains(&Correction::RemoveFact(CombatFact::FoodPickedUp {
                tick: 3,
                player: ALICE,
                from: bob_loot,
            }))
    );
    assert!(rpg.history_len::<Combatant>(BOB) > 0);
    assert!(rpg.history_len::<Inventory>(ALICE) > 0);
}

#[test]
fn duplicated_and_reordered_packets_are_deduped_before_simulation() {
    let mut rpg = NetworkedRpg::new(16);
    let cast = attack(7, 1, ALICE, BOB, 10);
    rpg.duplicate(cast, 3, 0);
    rpg.send(raise_shield(8, 1, BOB), 0);

    rpg.advance_to(4);

    assert_eq!(rpg.projectile_count(), 1);
    assert_eq!(rpg.hp(BOB), 100);
    assert!(rpg.rejected().contains(&RejectedInput::Duplicate {
        player: ALICE,
        sequence: 7,
    }));
    assert_eq!(
        rpg.facts()
            .iter()
            .filter(|fact| matches!(fact, CombatFact::SpellCast { caster, .. } if *caster == ALICE))
            .count(),
        1
    );
}

#[test]
fn dropped_then_resent_input_inside_rewind_window_still_corrects_state() {
    let mut rpg = NetworkedRpg::new(8);
    rpg.send(attack(1, 1, ALICE, BOB, 120), 0);
    rpg.drop_input(raise_shield(2, 1, BOB));

    rpg.advance_to(3);
    assert_eq!(rpg.hp(BOB), 0);

    rpg.send(raise_shield(3, 1, BOB), 5);
    rpg.advance_to(6);

    assert_eq!(rpg.hp(BOB), 100);
    assert_eq!(rpg.death_markers_for(BOB), 0);
    assert!(rpg.corrections().iter().any(|correction| matches!(
        correction,
        Correction::RemoveFact(CombatFact::PlayerDied { player, .. }) if *player == BOB
    )));
}

#[test]
fn late_input_at_retention_boundary_keeps_anchor_snapshot_for_replay() {
    let mut rpg = NetworkedRpg::new(3);
    rpg.send(attack(1, 1, ALICE, BOB, 120), 0);
    rpg.send(raise_shield(2, 1, BOB), 4);

    rpg.advance_to(5);

    assert_eq!(rpg.hp(BOB), 100);
    assert_eq!(rpg.death_markers_for(BOB), 0);
    assert!(rpg.facts().contains(&CombatFact::SpellBlocked {
        tick: 2,
        target: BOB,
    }));
    assert!(rpg.rejected().is_empty());
}

#[test]
fn late_same_tick_inputs_are_sorted_before_each_replay() {
    let mut rpg = NetworkedRpg::new(16);
    rpg.advance_to(3);

    rpg.receive_network_input(attack(2, 1, ALICE, CAROL, 120));
    rpg.receive_network_input(move_to(1, 1, ALICE, Vec3i::new(13, 0, 0)));

    assert_eq!(rpg.position(ALICE), Vec3i::new(13, 0, 0));
    assert_eq!(rpg.hp(CAROL), 0);
    assert!(rpg.facts().contains(&CombatFact::SpellCast {
        tick: 1,
        caster: ALICE,
        target: CAROL,
    }));
    assert!(!rpg.facts().contains(&CombatFact::SpellRejectedOutOfRange {
        tick: 1,
        caster: ALICE,
        target: CAROL,
    }));
}

#[test]
fn stale_late_input_outside_rewind_window_is_rejected_without_replay() {
    let mut rpg = NetworkedRpg::new(3);
    rpg.send(attack(1, 1, ALICE, BOB, 120), 0);
    rpg.send(raise_shield(2, 1, BOB), 8);

    rpg.advance_to(9);

    assert_eq!(rpg.hp(BOB), 0);
    assert_eq!(rpg.death_markers_for(BOB), 1);
    assert!(rpg.rejected().contains(&RejectedInput::Stale {
        player: BOB,
        sequence: 2,
        tick: 1,
    }));
    assert!(!rpg.corrections().iter().any(|correction| matches!(
        correction,
        Correction::RemoveFact(CombatFact::PlayerDied { player, .. }) if *player == BOB
    )));
}

#[test]
fn invalid_move_cannot_expand_spell_reach_under_latency() {
    let mut rpg = NetworkedRpg::new(16);
    rpg.send(move_to(1, 1, ALICE, Vec3i::new(10_000, 0, 0)), 2);
    rpg.send(attack(2, 2, ALICE, CAROL, 120), 2);

    rpg.advance_to(5);

    assert_eq!(rpg.position(ALICE), Vec3i::new(0, 0, 0));
    assert_eq!(rpg.hp(CAROL), 100);
    assert!(rpg.facts().contains(&CombatFact::MoveRejected {
        tick: 1,
        player: ALICE,
    }));
    assert!(rpg.facts().contains(&CombatFact::SpellRejectedOutOfRange {
        tick: 2,
        caster: ALICE,
        target: CAROL,
    }));
}
