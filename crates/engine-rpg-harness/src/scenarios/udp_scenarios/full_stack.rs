use super::*;
use std::{collections::HashMap, time::Duration};

struct InputChannel;

/// Client-side resource identifying which local entity the test controls.
#[derive(Resource)]
struct ControlledEntity(Entity);

/// Server-side resource mapping each client link entity to the authoritative
/// player entity.
#[derive(Resource)]
struct PlayerLinkMap(HashMap<Entity, Entity>);

fn register_full_stack(app: &mut App, _role: LightyearRole) {
    app.init_resource::<HistoryTick>();
    app.register_component::<StableEntityId>();
    app.register_component::<Health>().add_prediction();
    app.register_component::<CombatState>().add_prediction();
    app.register_component::<Transform>().add_prediction();

    app.add_channel::<InputChannel>(ChannelSettings {
        mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
        send_frequency: Duration::ZERO,
        priority: 1.0,
    })
    .add_direction(NetworkDirection::ClientToServer);
    app.register_message::<ActionState<AfterglowAction>>()
        .add_direction(NetworkDirection::ClientToServer);

    app.add_systems(PreUpdate, (client_send_input,));
    app.add_systems(
        PreUpdate,
        receive_client_input.after(ReplicationSystems::Receive),
    );
    app.add_systems(
        FixedUpdate,
        (
            advance_history_tick,
            resolve_shields,
            resolve_attacks,
            move_players,
        )
            .chain(),
    );
}

fn client_send_input(
    mut senders: Query<&mut MessageSender<ActionState<AfterglowAction>>>,
    controlled: Option<Res<ControlledEntity>>,
    players: Query<&ActionState<AfterglowAction>>,
) {
    let Some(controlled) = controlled else {
        return;
    };
    let Ok(state) = players.get(controlled.0) else {
        return;
    };
    for mut sender in senders.iter_mut() {
        sender.send::<InputChannel>(state.clone());
    }
}

fn receive_client_input(
    mut receivers: Query<(Entity, &mut MessageReceiver<ActionState<AfterglowAction>>)>,
    link_map: Option<Res<PlayerLinkMap>>,
    mut players: Query<&mut ActionState<AfterglowAction>>,
) {
    let Some(link_map) = link_map else {
        return;
    };
    for (link_entity, mut receiver) in receivers.iter_mut() {
        let Some(&player_entity) = link_map.0.get(&link_entity) else {
            continue;
        };
        for input in receiver.receive() {
            if let Ok(mut action_state) = players.get_mut(player_entity) {
                *action_state = input;
            }
        }
    }
}

fn udp_full_stack_rig(client_count: usize) -> LightyearTestRig {
    let mut rig = LightyearTestRig::new_with_transport(
        client_count,
        |_| {},
        register_full_stack,
        TransportConfig::Udp { server_port: 0 },
    );
    rig.connect();
    rig
}

fn advance_until(
    rig: &mut LightyearTestRig,
    max_ticks: u32,
    mut predicate: impl FnMut(&LightyearTestRig) -> bool,
    reason: &str,
) {
    for _ in 0..max_ticks {
        rig.advance(1);
        if predicate(rig) {
            return;
        }
    }
    panic!("condition not met after {max_ticks} ticks: {reason}");
}

fn init_player_link_map(rig: &mut LightyearTestRig) {
    rig.server_world_mut()
        .insert_resource(PlayerLinkMap(HashMap::new()));
}

fn setup_client_io(rig: &mut LightyearTestRig, client_id: usize, controlled_sid: StableEntityId) {
    let client_entity = rig.client_entity(controlled_sid, client_id);
    let client_link = rig.client_link(client_id);
    let server_link = rig.server_link(client_id);

    rig.client_world_mut(client_id)
        .entity_mut(client_link)
        .insert(MessageSender::<ActionState<AfterglowAction>>::default());
    rig.client_world_mut(client_id)
        .insert_resource(ControlledEntity(client_entity));

    rig.server_world_mut()
        .entity_mut(server_link)
        .insert(MessageReceiver::<ActionState<AfterglowAction>>::default());

    let server_entity = rig.server_entity(controlled_sid);
    let mut map = rig
        .server_world_mut()
        .get_resource_mut::<PlayerLinkMap>()
        .expect("PlayerLinkMap not initialized; call init_player_link_map first");
    map.0.insert(server_link, server_entity);
}

fn spawn_player(rig: &mut LightyearTestRig, sid: StableEntityId, pos: Vec3) -> Entity {
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

#[test]
fn udp_full_stack_movement_over_network() {
    let mut rig = udp_full_stack_rig(1);

    let alice_server = spawn_player(&mut rig, ALICE, Vec3::ZERO);
    let alice_client = rig.client_entity(ALICE, 0);

    init_player_link_map(&mut rig);
    setup_client_io(&mut rig, 0, ALICE);

    let mut state = ActionState::<AfterglowAction>::default();
    state.set_axis_pair(&AfterglowAction::Move, Vec2::new(0.0, 1.0));
    rig.client_world_mut(0)
        .entity_mut(alice_client)
        .insert(state);

    let pos_before = rig
        .server_component::<Transform>(alice_server)
        .unwrap()
        .translation;

    rig.advance(20);

    let pos_after = rig
        .server_component::<Transform>(alice_server)
        .unwrap()
        .translation;
    let moved = pos_after.distance(pos_before);
    assert!(
        moved > 0.2,
        "UDP full-stack: entity should move via network-delivered input: moved={moved}"
    );

    let client_pos = rig
        .client_component::<Transform>(0, alice_client)
        .unwrap()
        .translation;
    let client_moved = client_pos.distance(pos_before);
    assert!(
        client_moved > 0.2,
        "UDP full-stack: client entity should also move via prediction: moved={client_moved}"
    );
}

#[test]
fn udp_full_stack_combat_over_network() {
    let mut rig = udp_full_stack_rig(2);

    let alice_server = spawn_player(&mut rig, ALICE, Vec3::ZERO);
    let bob_server = spawn_player(&mut rig, BOB, Vec3::new(5.0, 0.0, 0.0));
    let alice_client = rig.client_entity(ALICE, 0);

    init_player_link_map(&mut rig);
    setup_client_io(&mut rig, 0, ALICE);

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
    rig.client_world_mut(0)
        .entity_mut(alice_client)
        .insert(state);

    advance_until(
        &mut rig,
        20,
        |rig| rig.server_component::<Health>(bob_server).unwrap().current == 100 - ATTACK_DAMAGE,
        "network-delivered attack should deal exactly one hit",
    );

    rig.client_world_mut(0)
        .entity_mut(alice_client)
        .insert(ActionState::<AfterglowAction>::default());
    rig.advance(5);

    let bob_hp = rig.server_component::<Health>(bob_server).unwrap().current;
    assert_eq!(
        bob_hp,
        100 - ATTACK_DAMAGE,
        "UDP full-stack: BOB should take exactly {ATTACK_DAMAGE} damage from one \
         network-delivered attack: hp={bob_hp}",
    );
}

#[test]
fn udp_full_stack_shield_blocks_attack() {
    let mut rig = udp_full_stack_rig(2);

    let alice_server = spawn_player(&mut rig, ALICE, Vec3::ZERO);
    let bob_server = spawn_player(&mut rig, BOB, Vec3::new(5.0, 0.0, 0.0));

    let bob_client = rig.client_entity(BOB, 1);

    init_player_link_map(&mut rig);
    setup_client_io(&mut rig, 0, ALICE);
    setup_client_io(&mut rig, 1, BOB);

    let mut bob_state = ActionState::<AfterglowAction>::default();
    bob_state.press(&AfterglowAction::RaiseShield);
    rig.client_world_mut(1)
        .entity_mut(bob_client)
        .insert(bob_state);

    advance_until(
        &mut rig,
        20,
        |rig| {
            rig.server_component::<CombatState>(bob_server)
                .unwrap()
                .shield_active_until
                > rig.server_world().resource::<HistoryTick>().0
        },
        "network-delivered shield should activate on the server",
    );

    let alice_client = rig.client_entity(ALICE, 0);
    let mut alice_state = ActionState::<AfterglowAction>::default();
    alice_state.press(&AfterglowAction::AttackPrimary);
    rig.client_world_mut(0)
        .entity_mut(alice_client)
        .insert(alice_state);

    advance_until(
        &mut rig,
        20,
        |rig| {
            rig.server_component::<ActionState<AfterglowAction>>(alice_server)
                .unwrap()
                .pressed(&AfterglowAction::AttackPrimary)
        },
        "network-delivered attack should arrive on the server",
    );

    assert!(
        rig.server_component::<CombatState>(bob_server)
            .unwrap()
            .shield_active_until
            > rig.server_world().resource::<HistoryTick>().0,
        "BOB shield input should activate before the attack resolves",
    );

    let bob_hp = rig.server_component::<Health>(bob_server).unwrap().current;
    assert_eq!(
        bob_hp, 100,
        "UDP full-stack: BOB should survive with shield blocking attack: hp={bob_hp}",
    );
    assert!(
        !rig.server_component::<CombatState>(bob_server)
            .unwrap()
            .dead,
        "BOB should not be dead",
    );
}
