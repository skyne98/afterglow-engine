//! UDP tests for Lightyear's Leafwing input plugin path.

use super::*;
use afterglow_engine::input::default_gameplay_input_map;
use bevy::time::Real;
use leafwing_input_manager::input_map::InputMap;
use lightyear::prelude::client::{InputDelayConfig, input::InputSystems};
use std::time::Duration;

#[derive(Resource)]
pub(crate) struct DesiredInput(pub(crate) ActionState<AfterglowAction>);

#[derive(Resource)]
pub(crate) struct InputEdgeProbe {
    pub(crate) entity: Entity,
    pub(crate) just_pressed_count: u32,
    pub(crate) just_released_count: u32,
    pub(crate) final_pressed: bool,
}

pub(crate) fn register_native_input(app: &mut App, role: LightyearRole) {
    app.init_resource::<HistoryTick>();
    app.register_component::<StableEntityId>();
    app.register_component::<Health>().add_prediction();
    app.register_component::<CombatState>().add_prediction();
    app.register_component::<Transform>().add_prediction();

    if matches!(role, LightyearRole::Client | LightyearRole::Host) {
        app.add_plugins(bevy::input::InputPlugin);
    }
    app.add_plugins(lightyear::prelude::input::leafwing::InputPlugin::<
        AfterglowAction,
    >::default());

    if matches!(role, LightyearRole::Client | LightyearRole::Host) {
        app.add_systems(
            FixedPreUpdate,
            apply_desired_input.in_set(InputSystems::WriteClientInputs),
        );
    }

    app.add_systems(
        FixedUpdate,
        (
            advance_history_tick,
            resolve_shields,
            resolve_attacks,
            move_players,
            probe_input_edges,
        )
            .chain(),
    );
}

fn apply_desired_input(
    desired: Option<Res<DesiredInput>>,
    mut query: Query<&mut ActionState<AfterglowAction>, With<InputMap<AfterglowAction>>>,
) {
    let Some(desired) = desired else {
        return;
    };
    for mut state in &mut query {
        *state = desired.0.clone();
    }
}

fn probe_input_edges(
    query: Query<&ActionState<AfterglowAction>>,
    mut probe: Option<ResMut<InputEdgeProbe>>,
) {
    let Some(ref mut probe) = probe else { return };
    let Ok(state) = query.get(probe.entity) else {
        return;
    };
    if state.just_pressed(&AfterglowAction::Jump) {
        probe.just_pressed_count += 1;
    }
    if state.just_released(&AfterglowAction::Jump) {
        probe.just_released_count += 1;
    }
    probe.final_pressed = state.pressed(&AfterglowAction::Jump);
}

fn udp_native_input_rig(client_count: usize) -> LightyearTestRig {
    let mut rig = LightyearTestRig::new_with_transport(
        client_count,
        |_| {},
        register_native_input,
        TransportConfig::Udp { server_port: 0 },
    );
    rig.connect();
    set_fixed_native_input_delay(&mut rig, 2);
    wait_for_native_input_sync(&mut rig);
    rig
}

pub(crate) fn set_fixed_native_input_delay(rig: &mut LightyearTestRig, delay_ticks: u16) {
    for client_id in 0..rig.client_apps.len() {
        let client_link = rig.client_link(client_id);
        rig.client_world_mut(client_id)
            .entity_mut(client_link)
            .insert(
                InputTimelineConfig::default()
                    .with_input_delay(InputDelayConfig::fixed_input_delay(delay_ticks)),
            );
    }
}

pub(crate) fn wait_for_native_input_sync(rig: &mut LightyearTestRig) {
    for _ in 0..240 {
        if (0..rig.client_apps.len()).all(|i| native_input_synced(rig, i)) {
            return;
        }
        rig.advance(1);
    }
    panic!("native input timelines did not sync");
}

fn native_input_synced(rig: &LightyearTestRig, client_id: usize) -> bool {
    let client_link = rig.client_link(client_id);
    rig.client_world(client_id)
        .get::<IsSynced<InputTimeline>>(client_link)
        .is_some()
}

pub(crate) fn spawn_player(rig: &mut LightyearTestRig, sid: StableEntityId, pos: Vec3) -> Entity {
    let entity = rig.spawn_replicated(
        sid,
        (
            Health {
                current: 100,
                max: 100,
            },
            CombatState::default(),
            Transform::from_translation(pos),
            ActionState::<AfterglowAction>::default(),
        ),
    );
    let mut entities = vec![entity];
    for i in 0..rig.client_apps.len() {
        let c = rig
            .find_client_entity(i, sid)
            .unwrap_or_else(|| panic!("client {i} entity for {sid:?}"));
        entities.push(c);
    }
    rig.register_entity(sid, entities);
    entity
}

pub(crate) fn setup_client_native_input(
    rig: &mut LightyearTestRig,
    client_id: usize,
    sid: StableEntityId,
) {
    let entity = rig.client_entity(sid, client_id);
    rig.client_world_mut(client_id)
        .entity_mut(entity)
        .insert(default_gameplay_input_map());

    let client_link = rig.client_link(client_id);
    let remote_entity = rig
        .client_world(client_id)
        .get::<MessageManager>(client_link)
        .expect("client link should have MessageManager")
        .entity_mapper
        .get_remote(entity);
    assert!(
        remote_entity.is_some(),
        "client entity {entity:?} should map back to the server entity for {sid:?}"
    );
    assert!(
        rig.client_world(client_id)
            .get::<lightyear::prelude::input::leafwing::LeafwingBuffer<AfterglowAction>>(entity)
            .is_some(),
        "client entity should have native Leafwing input buffer after InputMap insertion"
    );
}

pub(crate) fn assert_native_input_link_ready(rig: &LightyearTestRig, client_id: usize) {
    let client_link = rig.client_link(client_id);
    let client_components = component_names(rig.client_world(client_id), client_link);
    assert!(
        client_components
            .iter()
            .any(|name| name.contains("MessageSender") && name.contains("InputMessage")),
        "client link should have native InputMessage sender; components={client_components:?}"
    );
    let client_transport = rig
        .client_world(client_id)
        .get::<Transport>(client_link)
        .expect("client link should have Transport");
    assert!(
        client_transport.has_sender::<lightyear::input::InputChannel>(),
        "client link should have native input channel sender"
    );
    assert!(
        rig.client_world(client_id)
            .get::<IsSynced<InputTimeline>>(client_link)
            .is_some(),
        "client link should have a synced input timeline"
    );

    let server_link = rig.server_link(client_id);
    let server_components = component_names(rig.server_world(), server_link);
    assert!(
        server_components
            .iter()
            .any(|name| name.contains("MessageReceiver") && name.contains("InputMessage")),
        "server link should have native InputMessage receiver; components={server_components:?}"
    );
    let server_transport = rig
        .server_world()
        .get::<Transport>(server_link)
        .expect("server link should have Transport");
    assert!(
        server_transport.has_receiver::<lightyear::input::InputChannel>(),
        "server link should have native input channel receiver"
    );
}

fn component_names(world: &World, entity: Entity) -> Vec<String> {
    let entity_ref = world.entity(entity);
    entity_ref
        .archetype()
        .components()
        .iter()
        .copied()
        .filter_map(|component| world.components().get_info(component))
        .map(|info| info.name().to_string())
        .collect()
}

pub(crate) fn set_desired_input(
    rig: &mut LightyearTestRig,
    client_id: usize,
    state: ActionState<AfterglowAction>,
) {
    rig.client_world_mut(client_id)
        .insert_resource(DesiredInput(state));
}

pub(crate) fn advance_until(
    rig: &mut LightyearTestRig,
    max: u32,
    mut predicate: impl FnMut(&LightyearTestRig) -> bool,
    reason: &str,
) {
    for _ in 0..max {
        rig.advance(1);
        if predicate(rig) {
            return;
        }
    }
    panic!("condition not met after {max} ticks: {reason}");
}

pub(crate) fn native_input_movement_body(rig: &mut LightyearTestRig) {
    let alice_server = spawn_player(rig, ALICE, Vec3::ZERO);
    setup_client_native_input(rig, 0, ALICE);
    assert_native_input_link_ready(rig, 0);

    let pos_before = rig
        .server_component::<Transform>(alice_server)
        .unwrap()
        .translation;

    let mut state = ActionState::<AfterglowAction>::default();
    state.set_axis_pair(&AfterglowAction::Move, Vec2::new(0.0, 1.0));
    set_desired_input(rig, 0, state);

    rig.advance(20);

    let pos_after = rig
        .server_component::<Transform>(alice_server)
        .unwrap()
        .translation;
    let moved = pos_after.distance(pos_before);
    assert!(
        moved > 0.2,
        "native input should move the server entity: moved={moved}"
    );

    let alice_client = rig.client_entity(ALICE, 0);
    let client_pos = rig
        .client_component::<Transform>(0, alice_client)
        .unwrap()
        .translation;
    let client_moved = client_pos.distance(pos_before);
    assert!(
        client_moved > 0.2,
        "native input should move the predicted client entity: moved={client_moved}"
    );
}

pub(crate) fn native_input_combat_body(rig: &mut LightyearTestRig) {
    let alice_server = spawn_player(rig, ALICE, Vec3::ZERO);
    let bob_server = spawn_player(rig, BOB, Vec3::new(5.0, 0.0, 0.0));
    setup_client_native_input(rig, 0, ALICE);
    assert_native_input_link_ready(rig, 0);

    rig.server_world_mut().insert_resource(AttackCooldown(10));
    rig.advance(10);

    let tick = rig.server_world().resource::<HistoryTick>().0;
    rig.server_world_mut()
        .entity_mut(alice_server)
        .get_mut::<CombatState>()
        .expect("ALICE should have CombatState")
        .last_attack_tick = tick - 10;

    let mut state = ActionState::<AfterglowAction>::default();
    state.press(&AfterglowAction::AttackPrimary);
    set_desired_input(rig, 0, state);

    advance_until(
        rig,
        20,
        |r| r.server_component::<Health>(bob_server).unwrap().current == 100 - ATTACK_DAMAGE,
        "native input attack should deal exactly one hit",
    );

    set_desired_input(rig, 0, ActionState::<AfterglowAction>::default());
    rig.advance(5);

    let bob_hp = rig.server_component::<Health>(bob_server).unwrap().current;
    assert_eq!(
        bob_hp,
        100 - ATTACK_DAMAGE,
        "BOB should take exactly {ATTACK_DAMAGE} damage from native input: hp={bob_hp}",
    );
}

pub(crate) fn native_input_shield_body(rig: &mut LightyearTestRig) {
    let alice_server = spawn_player(rig, ALICE, Vec3::ZERO);
    let bob_server = spawn_player(rig, BOB, Vec3::new(5.0, 0.0, 0.0));

    setup_client_native_input(rig, 1, BOB);
    assert_native_input_link_ready(rig, 1);
    let mut bob_state = ActionState::<AfterglowAction>::default();
    bob_state.press(&AfterglowAction::RaiseShield);
    set_desired_input(rig, 1, bob_state);

    // The system chain renews held shields before attacks are resolved.
    advance_until(
        rig,
        20,
        |r| {
            r.server_component::<CombatState>(bob_server)
                .unwrap()
                .shield_active_until
                > r.server_world().resource::<HistoryTick>().0
        },
        "native input shield should activate on the server",
    );

    setup_client_native_input(rig, 0, ALICE);
    assert_native_input_link_ready(rig, 0);
    let mut alice_state = ActionState::<AfterglowAction>::default();
    alice_state.press(&AfterglowAction::AttackPrimary);
    set_desired_input(rig, 0, alice_state);

    advance_until(
        rig,
        20,
        |r| {
            r.server_component::<ActionState<AfterglowAction>>(alice_server)
                .is_some_and(|state| state.pressed(&AfterglowAction::AttackPrimary))
        },
        "native input attack should arrive on the server",
    );

    let bob_hp = rig.server_component::<Health>(bob_server).unwrap().current;
    assert_eq!(
        bob_hp, 100,
        "BOB shield should block the attack: hp={bob_hp}"
    );
    assert!(
        !rig.server_component::<CombatState>(bob_server)
            .unwrap()
            .dead,
        "BOB should not be dead"
    );
}

pub(crate) fn native_input_edges_body(rig: &mut LightyearTestRig) {
    let alice_server = spawn_player(rig, ALICE, Vec3::ZERO);
    setup_client_native_input(rig, 0, ALICE);
    assert_native_input_link_ready(rig, 0);
    rig.server_world_mut().insert_resource(InputEdgeProbe {
        entity: alice_server,
        just_pressed_count: 0,
        just_released_count: 0,
        final_pressed: false,
    });

    let mut press = ActionState::<AfterglowAction>::default();
    press.press(&AfterglowAction::Jump);
    set_desired_input(rig, 0, press);
    rig.advance(1);

    let mut held = ActionState::<AfterglowAction>::default();
    held.press(&AfterglowAction::Jump);

    // Derive time from the client's deterministic clock to avoid
    // wall-clock nondeterminism.  The clock's startup Instant is captured
    // once at app creation; elapsed advances by exactly 1/60 s per rig
    // tick, so (startup + elapsed) changes deterministically across runs.
    let time = rig.client_world(0).resource::<Time<Real>>();
    let current = time.startup() + time.elapsed();
    let previous = current - Duration::from_secs_f64(1.0 / 60.0);
    held.tick(current, previous);

    set_desired_input(rig, 0, held.clone());

    advance_until(
        rig,
        20,
        |r| {
            r.server_world()
                .resource::<InputEdgeProbe>()
                .just_pressed_count
                == 1
        },
        "native input jump press edge should arrive once",
    );

    let mut release = held;
    release.release(&AfterglowAction::Jump);
    set_desired_input(rig, 0, release);
    rig.advance(1);

    set_desired_input(rig, 0, ActionState::<AfterglowAction>::default());
    advance_until(
        rig,
        20,
        |r| {
            r.server_world()
                .resource::<InputEdgeProbe>()
                .just_released_count
                == 1
        },
        "native input jump release edge should arrive once",
    );
    rig.advance(5);

    let probe = rig.server_world().resource::<InputEdgeProbe>();
    assert_eq!(probe.just_pressed_count, 1);
    assert_eq!(probe.just_released_count, 1);
    assert!(
        !probe.final_pressed,
        "final server state should be released"
    );
}

#[test]
fn udp_native_input_movement_over_network() {
    let mut rig = udp_native_input_rig(1);
    native_input_movement_body(&mut rig);
}

#[test]
fn udp_native_input_combat_over_network() {
    let mut rig = udp_native_input_rig(2);
    native_input_combat_body(&mut rig);
}

#[test]
fn udp_native_input_shield_blocks_attack() {
    let mut rig = udp_native_input_rig(2);
    native_input_shield_body(&mut rig);
}

#[test]
fn udp_native_input_edges_arrive_once() {
    let mut rig = udp_native_input_rig(1);
    native_input_edges_body(&mut rig);
}
