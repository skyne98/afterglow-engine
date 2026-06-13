use super::{components::*, systems::*};
use crate::rig::LightyearTestRig;
use afterglow_engine::{
    core::identity::StableEntityId,
    input::AfterglowAction,
    network::{LightyearRole, register_afterglow_lightyear_protocol},
    physics::{
        AfterglowPhysicsPlugin,
        avian::{Collider, Gravity, RigidBody},
    },
};
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::*;

const DOOR: StableEntityId = StableEntityId::from_raw(1);
const PLAYER: StableEntityId = StableEntityId::from_raw(2);

fn player_bundle(pos: Vec3) -> impl Bundle {
    (
        Health {
            current: 100,
            max: 100,
        },
        Transform::from_translation(pos),
        ActionState::<AfterglowAction>::default(),
        RigidBody::Dynamic,
        Collider::sphere(0.3),
    )
}

fn door_bundle(pos: Vec3, open: bool, locked: bool) -> impl Bundle {
    (
        DoorState { open, locked },
        Transform::from_translation(pos),
        RigidBody::Kinematic,
        Collider::cuboid(0.5, 1.0, 0.05),
    )
}

fn register_doors(app: &mut App, _role: LightyearRole) {
    register_afterglow_lightyear_protocol(app);
    app.register_component::<Health>().add_prediction();
    app.register_component::<DoorState>().add_prediction();
    app.register_component::<DoorGrab>().add_prediction();
    app.add_systems(
        FixedUpdate,
        (resolve_door_interactions, apply_door_open).chain(),
    );
}

fn zero_gravity(app: &mut App) {
    if let Some(mut gravity) = app.world_mut().get_resource_mut::<Gravity>() {
        gravity.0 = Vec3::ZERO;
    }
}

#[test]
fn door_opens_on_grab() {
    let mut rig = LightyearTestRig::new(
        1,
        |app| {
            app.add_plugins(AfterglowPhysicsPlugin);
        },
        register_doors,
    );

    zero_gravity(&mut rig.server_app);
    for client in &mut rig.client_apps {
        zero_gravity(client);
    }

    let door = rig.spawn_replicated(DOOR, door_bundle(Vec3::new(0.0, 0.0, -3.0), false, false));
    let player = rig.spawn_replicated(PLAYER, player_bundle(Vec3::ZERO));

    let player_c0 = rig
        .find_client_entity(0, PLAYER)
        .expect("PLAYER on client 0");
    let door_c0 = rig.find_client_entity(0, DOOR).expect("DOOR on client 0");
    rig.register_entity(PLAYER, vec![player, player_c0]);
    rig.register_entity(DOOR, vec![door, door_c0]);

    let client_link = rig.client_link(0);
    let hash = door_grab_hash(PLAYER, DOOR);

    let predicted_grab = rig
        .client_world_mut(0)
        .spawn((
            DoorGrab {
                player: PLAYER,
                door: DOOR,
            },
            PreSpawned::new(hash).for_receiver(client_link),
        ))
        .id();

    rig.queue_action(1, {
        let player = player;
        move |app| {
            let mut state = ActionState::<AfterglowAction>::default();
            state.press(&AfterglowAction::Use);
            app.world_mut().entity_mut(player).insert(state);
        }
    });
    rig.queue_action(2, {
        let player = player;
        move |app| {
            app.world_mut()
                .entity_mut(player)
                .insert(ActionState::<AfterglowAction>::default());
        }
    });

    let player_start_z = rig
        .server_component::<Transform>(player)
        .unwrap()
        .translation
        .z;

    rig.advance_to(30);

    assert!(
        rig.client_world(0).get_entity(predicted_grab).is_ok(),
        "DoorGrab entity should exist on client after server confirmation"
    );

    let door_state = rig.server_component::<DoorState>(door).unwrap();
    assert!(door_state.open, "Door should be open after confirmed grab");

    let player_end_z = rig
        .server_component::<Transform>(player)
        .unwrap()
        .translation
        .z;
    assert!(
        player_end_z < player_start_z,
        "Player should be pulled toward door (z decreased): start={}, end={}",
        player_start_z,
        player_end_z,
    );
}

#[test]
fn locked_door_rejects_grab_and_cleans_up() {
    let mut rig = LightyearTestRig::new(
        1,
        |app| {
            app.add_plugins(AfterglowPhysicsPlugin);
        },
        register_doors,
    );

    zero_gravity(&mut rig.server_app);
    for client in &mut rig.client_apps {
        zero_gravity(client);
    }

    let _door = rig.spawn_replicated(DOOR, door_bundle(Vec3::new(0.0, 0.0, -3.0), false, true));
    let _player = rig.spawn_replicated(PLAYER, player_bundle(Vec3::ZERO));

    let player_c0 = rig
        .find_client_entity(0, PLAYER)
        .expect("PLAYER on client 0");
    let door_c0 = rig.find_client_entity(0, DOOR).expect("DOOR on client 0");
    rig.register_entity(PLAYER, vec![_player, player_c0]);
    rig.register_entity(DOOR, vec![_door, door_c0]);

    let client_link = rig.client_link(0);
    let hash = door_grab_hash(PLAYER, DOOR);

    let predicted_grab = rig
        .client_world_mut(0)
        .spawn((
            DoorGrab {
                player: PLAYER,
                door: DOOR,
            },
            PreSpawned::new(hash).for_receiver(client_link),
        ))
        .id();

    rig.advance(80);

    assert!(
        rig.client_world(0).get_entity(predicted_grab).is_err(),
        "Unmatched DoorGrab PreSpawned entity should be despawned after timeout"
    );

    let door_state = rig.server_component::<DoorState>(_door).unwrap();
    assert!(!door_state.open, "Locked door should remain closed");
    assert!(door_state.locked, "Door should still be locked");
}
