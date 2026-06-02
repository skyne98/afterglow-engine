use super::{components::*, systems::*};
use crate::rig::{LightyearTestRig, TransportConfig};
use afterglow_engine::{
    controller::{
        AfterglowFirstPersonControllerPlugin, ControllerStance, FirstPersonController,
        FirstPersonControllerConfig, FirstPersonEffectStack, FirstPersonImpulseBuffer,
    },
    core::identity::StableEntityId,
    input::AfterglowAction,
    network::{HistoryTick, LightyearRole},
    physics::AfterglowPhysicsPlugin,
};
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::*;

const ALICE: StableEntityId = StableEntityId::from_raw(1);
const BOB: StableEntityId = StableEntityId::from_raw(2);

fn reconcile_controller_components(
    mut effects: Query<
        (
            &mut FirstPersonEffectStack,
            &Confirmed<FirstPersonEffectStack>,
        ),
        With<Predicted>,
    >,
    mut impulses: Query<
        (
            &mut FirstPersonImpulseBuffer,
            &Confirmed<FirstPersonImpulseBuffer>,
        ),
        With<Predicted>,
    >,
) {
    for (mut predicted, confirmed) in &mut effects {
        if *predicted != confirmed.0 {
            *predicted = confirmed.0.clone();
        }
    }
    for (mut predicted, confirmed) in &mut impulses {
        if *predicted != confirmed.0 {
            *predicted = confirmed.0;
        }
    }
}

fn register_gold(app: &mut App, _role: LightyearRole) {
    app.init_resource::<HistoryTick>();
    app.register_component::<StableEntityId>();
    app.register_component::<Health>().add_prediction();
    app.register_component::<CombatState>().add_prediction();
    app.register_component::<FirstPersonEffectStack>()
        .add_prediction();
    app.register_component::<FirstPersonImpulseBuffer>()
        .add_prediction();
    app.register_component::<Corpse>();
    app.register_component::<Loot>().add_prediction();
    app.add_systems(
        FixedUpdate,
        (
            advance_history_tick,
            sync_dead_state,
            resolve_shields,
            resolve_attacks,
            apply_deaths,
            resolve_loot_pickup,
        )
            .chain(),
    );
    app.add_systems(
        PreUpdate,
        reconcile_controller_components.after(ReplicationSystems::Receive),
    );
}

fn player_bundle(pos: Vec3) -> impl Bundle {
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    (
        Health {
            current: 100,
            max: 100,
        },
        CombatState::default(),
        FirstPersonController { config },
        Transform::from_translation(pos + Vec3::Y * half_height),
        ActionState::<AfterglowAction>::default(),
    )
}

fn set_action_state(world: &mut World, entity: Entity, action: AfterglowAction) {
    let mut state = ActionState::<AfterglowAction>::default();
    state.press(&action);
    world.entity_mut(entity).insert(state);
}

#[test]
fn alice_attacks_bob_shield_blocks() {
    let mut rig = LightyearTestRig::new(
        2,
        |app| {
            app.add_plugins(AfterglowPhysicsPlugin);
            app.add_plugins(AfterglowFirstPersonControllerPlugin);
        },
        register_gold,
    )
    .with_input_delay_ms(50);

    // Spawn BOTH players via Lightyear replication so entities appear on clients
    let alice = rig.spawn_replicated(ALICE, player_bundle(Vec3::ZERO));
    let bob = rig.spawn_replicated(BOB, player_bundle(Vec3::new(5.0, 0.0, 0.0)));

    // Find client-side entity IDs (StableEntityId is registered for replication)
    let alice_c0 = rig.find_client_entity(0, ALICE).expect("ALICE on client 0");
    let alice_c1 = rig.find_client_entity(1, ALICE).expect("ALICE on client 1");
    let bob_c0 = rig.find_client_entity(0, BOB).expect("BOB on client 0");
    let bob_c1 = rig.find_client_entity(1, BOB).expect("BOB on client 1");

    rig.register_entity(ALICE, vec![alice, alice_c0, alice_c1]);
    rig.register_entity(BOB, vec![bob, bob_c0, bob_c1]);

    // Client-side prediction: set ActionState on client replicated entities
    set_action_state(
        rig.client_world_mut(0),
        alice_c0,
        AfterglowAction::AttackPrimary,
    );
    set_action_state(
        rig.client_world_mut(1),
        bob_c1,
        AfterglowAction::RaiseShield,
    );

    // Server-side delayed input:
    // Tick 1: BOB raises shield (delayed by ~3 ticks → arrives at tick 4)
    rig.queue_action(1, move |app| {
        set_action_state(app.world_mut(), bob, AfterglowAction::RaiseShield);
    });
    // Tick 2: ALICE attacks (delayed by ~3 ticks → arrives at tick 5)
    rig.queue_action(2, move |app| {
        set_action_state(app.world_mut(), alice, AfterglowAction::AttackPrimary);
    });

    // Advance to tick 5 — server processes tick 1 (shield) at tick 4, tick 2
    // (attack) at tick 5
    rig.advance_to(5);

    // Server state: BOB alive, shield blocked
    let bob_hp_server = rig.server_component::<Health>(bob).unwrap().current;
    assert_eq!(
        bob_hp_server, 100,
        "BOB should be alive (shield blocked attack) on server"
    );

    let bob_combat_server = rig.server_component::<CombatState>(bob).unwrap();
    assert!(!bob_combat_server.dead, "BOB should not be dead on server");

    // No corpse or loot on server — BOB survived so no death occurred
    assert!(
        rig.server_component::<Corpse>(bob).is_none(),
        "No corpse should exist on server since BOB survived"
    );
    assert!(
        rig.server_component::<Loot>(bob).is_none(),
        "No loot should exist on server since BOB survived"
    );

    // Client 0 (ALICE's client) state: BOB alive, replicated from server
    let bob_hp_client0 = rig.client_component::<Health>(0, bob_c0).unwrap().current;
    assert_eq!(bob_hp_client0, 100, "BOB should be alive on ALICE's client");

    let bob_combat_client0 = rig.client_component::<CombatState>(0, bob_c0).unwrap();
    assert!(
        !bob_combat_client0.dead,
        "BOB should not be dead on ALICE's client"
    );

    // Client state matches server state (proves replication worked)
    let bob_hp_server = rig.server_component::<Health>(bob).unwrap();
    let bob_hp_client0 = rig.client_component::<Health>(0, bob_c0).unwrap();
    assert_eq!(
        bob_hp_server, bob_hp_client0,
        "Client 0's view of BOB's Health should match server"
    );

    // Client 1 (BOB's client) state: BOB alive, shield active
    let bob_hp_client1 = rig.client_component::<Health>(1, bob_c1).unwrap().current;
    assert_eq!(bob_hp_client1, 100, "BOB should be alive on his own client");
}

/// UDP gold scenario — proves entity replication + combat over real sockets.
#[test]
fn shield_blocks_attack_over_udp() {
    let mut rig = LightyearTestRig::new_with_transport(
        2,
        |app| {
            app.add_plugins(AfterglowPhysicsPlugin);
            app.add_plugins(AfterglowFirstPersonControllerPlugin);
        },
        register_gold,
        TransportConfig::Udp { server_port: 0 },
    )
    .with_input_delay_ms(50);
    rig.connect();

    // Spawn BOTH players via Lightyear replication so entities appear on clients
    let alice = rig.spawn_replicated(ALICE, player_bundle(Vec3::ZERO));
    let bob = rig.spawn_replicated(BOB, player_bundle(Vec3::new(5.0, 0.0, 0.0)));

    // Find client-side entity IDs (StableEntityId is registered for replication)
    let alice_c0 = rig.find_client_entity(0, ALICE).expect("ALICE on client 0");
    let alice_c1 = rig.find_client_entity(1, ALICE).expect("ALICE on client 1");
    let bob_c0 = rig.find_client_entity(0, BOB).expect("BOB on client 0");
    let bob_c1 = rig.find_client_entity(1, BOB).expect("BOB on client 1");

    rig.register_entity(ALICE, vec![alice, alice_c0, alice_c1]);
    rig.register_entity(BOB, vec![bob, bob_c0, bob_c1]);

    // Verify initial replicated state matches server
    assert_eq!(
        rig.server_component::<Health>(alice).unwrap(),
        rig.client_component::<Health>(0, alice_c0).unwrap(),
        "ALICE HP client 0 matches server"
    );
    assert_eq!(
        rig.server_component::<Health>(bob).unwrap(),
        rig.client_component::<Health>(1, bob_c1).unwrap(),
        "BOB HP client 1 matches server"
    );

    // Queue combat actions (work over UDP transport)
    rig.queue_action(1, move |app| {
        set_action_state(app.world_mut(), bob, AfterglowAction::RaiseShield);
    });
    rig.queue_action(2, move |app| {
        set_action_state(app.world_mut(), alice, AfterglowAction::AttackPrimary);
    });

    rig.advance(10);

    // Server: shield blocks attack (tick ordering works over UDP)
    assert_eq!(rig.server_component::<Health>(bob).unwrap().current, 100);
    assert!(!rig.server_component::<CombatState>(bob).unwrap().dead);
    assert!(rig.server_component::<Corpse>(bob).is_none());
    assert!(rig.server_component::<Loot>(bob).is_none());

    // Client states match server after replication
    let bob_hp_server = rig.server_component::<Health>(bob).unwrap();
    let bob_hp_client0 = rig.client_component::<Health>(0, bob_c0).unwrap();
    assert_eq!(
        bob_hp_server, bob_hp_client0,
        "UDP: Client 0's view of BOB's Health should match server"
    );
    let bob_hp_client1 = rig.client_component::<Health>(1, bob_c1).unwrap();
    assert_eq!(
        bob_hp_server, bob_hp_client1,
        "UDP: Client 1's view of BOB's Health should match server"
    );
}
