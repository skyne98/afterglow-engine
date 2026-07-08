//! UDP/netcode regression for multiplayer_boxes using the production
//! `AfterglowLightyearPlugin` + `AfterglowConnectionPlugin` path.
//!
//! This catches live-runtime gaps that the Crossbeam harness can mask, such as
//! missing client `Transport` channels or missing input timeline sync markers.

use crate::rig::LightyearTestRig;
use afterglow_engine::{
    demos::multiplayer_boxes::{
        movement::{
            DemoInput, add_input_map_to_local_predicted_player, apply_movement,
            apply_predicted_movement, collect_input,
        },
        network::register_demo_protocol,
        protocol::{PLAYER_SIZE, PlayerBox},
    },
    input::AfterglowAction,
    network::{
        LightyearRole,
        connection::{
            AuthResponse, ChallengeMessage, ConnectionEvent, ConnectionEventKind, PlayerOwned,
        },
    },
};
use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::{
    ActionsChannel, MessageReceiver, MessageSender, NetworkTarget, Predicted, PredictionTarget,
    Replicate, ReplicationSystems, client::input::InputSystems,
};
use lightyear_inputs_leafwing::prelude::LeafwingBuffer;

const ALICE_ID: u64 = 1;
const BOB_ID: u64 = 2;

fn spawn_player_on_connected_for_test(trigger: On<ConnectionEvent>, mut commands: Commands) {
    let ConnectionEventKind::Connected = trigger.event().kind else {
        return;
    };
    let player_id = trigger.event().player_id;
    commands.spawn((
        PlayerBox {
            owner: player_id.to_string(),
        },
        Transform::from_translation(Vec3::new(0.0, PLAYER_SIZE, 0.0)),
        ActionState::<AfterglowAction>::default(),
        LeafwingBuffer::<AfterglowAction>::default(),
        LinearVelocity::ZERO,
        PlayerOwned::from_player_id(player_id),
        Replicate::to_clients(NetworkTarget::All),
        PredictionTarget::to_clients(NetworkTarget::All),
    ));
}

fn integrate_velocity_for_test(
    mut players: Query<(&mut Transform, &LinearVelocity), With<PlayerBox>>,
) {
    const FIXED_DT: f32 = 1.0 / 60.0;
    for (mut transform, velocity) in &mut players {
        transform.translation += velocity.0 * FIXED_DT;
    }
}

fn register_boxes(app: &mut App, role: LightyearRole) {
    register_demo_protocol(app);
    match role {
        LightyearRole::Server => {
            app.add_observer(spawn_player_on_connected_for_test);
            app.add_systems(
                FixedUpdate,
                (apply_movement, integrate_velocity_for_test).chain(),
            );
        }
        LightyearRole::Client => {
            app.init_resource::<DemoInput>();
            app.add_systems(
                PreUpdate,
                add_input_map_to_local_predicted_player.after(ReplicationSystems::Receive),
            );
            app.add_systems(
                FixedPreUpdate,
                collect_input.in_set(InputSystems::WriteClientInputs),
            );
            app.add_systems(
                FixedUpdate,
                (apply_predicted_movement, integrate_velocity_for_test).chain(),
            );
        }
    }
}

fn find_server_player(rig: &mut LightyearTestRig, owner: u64) -> Option<Entity> {
    let world = rig.server_world_mut();
    let mut q = world.query::<(Entity, &PlayerBox)>();
    q.iter(world)
        .find_map(|(entity, player)| (player.owner == owner.to_string()).then_some(entity))
}

fn find_predicted_player(
    rig: &mut LightyearTestRig,
    client_id: usize,
    owner: u64,
) -> Option<Entity> {
    let world = rig.client_world_mut(client_id);
    let mut q = world.query_filtered::<(Entity, &PlayerBox), With<Predicted>>();
    q.iter(world)
        .find_map(|(entity, player)| (player.owner == owner.to_string()).then_some(entity))
}

fn visible_pos(rig: &LightyearTestRig, client_id: usize, entity: Entity) -> Vec3 {
    rig.client_world(client_id)
        .get::<Transform>(entity)
        .expect("client player should have Transform")
        .translation
}

fn server_pos(rig: &LightyearTestRig, entity: Entity) -> Vec3 {
    rig.server_world()
        .get::<Transform>(entity)
        .expect("server player should have Transform")
        .translation
}

fn set_client_move(rig: &mut LightyearTestRig, client_id: usize, dir: Vec2) {
    rig.client_world_mut(client_id)
        .resource_mut::<DemoInput>()
        .0 = dir;
    if let Some(mut keys) = rig
        .client_world_mut(client_id)
        .get_resource_mut::<ButtonInput<KeyCode>>()
    {
        for key in [KeyCode::KeyW, KeyCode::KeyA, KeyCode::KeyS, KeyCode::KeyD] {
            keys.release(key);
        }
        if dir.y > 0.0 {
            keys.press(KeyCode::KeyW);
        }
        if dir.y < 0.0 {
            keys.press(KeyCode::KeyS);
        }
        if dir.x > 0.0 {
            keys.press(KeyCode::KeyA);
        }
        if dir.x < 0.0 {
            keys.press(KeyCode::KeyD);
        }
    }
}

#[test]
fn udp_afterglow_connection_plugin_syncs_player_input_and_visible_remote_transform() {
    let mut rig = LightyearTestRig::new_afterglow_udp(
        2,
        |app| {
            app.add_plugins((bevy::input::InputPlugin, bevy::transform::TransformPlugin));
        },
        register_boxes,
        0,
    );

    rig.connect();

    let client0_link = rig.client_link(0);
    let server0_link = rig.server_link(0);

    rig.server_world_mut()
        .get_mut::<MessageSender<ChallengeMessage>>(server0_link)
        .expect("server link should have ChallengeMessage sender")
        .send::<ActionsChannel>(ChallengeMessage { nonce: [1; 32] });
    rig.advance(10);
    let direct_challenge_count = rig
        .client_world_mut(0)
        .get_mut::<MessageReceiver<ChallengeMessage>>(client0_link)
        .expect("client link should have ChallengeMessage receiver")
        .receive()
        .count();

    rig.client_world_mut(0)
        .get_mut::<MessageSender<AuthResponse>>(client0_link)
        .expect("client link should have AuthResponse sender")
        .send::<ActionsChannel>(AuthResponse {
            public_key: [0; 32],
            signature: Vec::new(),
        });
    rig.advance(50);
    let mut direct_auth_count = 0usize;
    for link_index in 0..2 {
        let link = rig.server_link(link_index);
        direct_auth_count += rig
            .server_world_mut()
            .get_mut::<MessageReceiver<AuthResponse>>(link)
            .expect("server link should have AuthResponse receiver")
            .receive()
            .count();
    }
    assert_eq!(
        direct_challenge_count, 1,
        "production UDP path must deliver server-to-client Lightyear messages"
    );
    assert_eq!(
        direct_auth_count, 1,
        "production UDP path must deliver client-to-server Lightyear messages"
    );

    rig.advance(20);

    let server_alice = find_server_player(&mut rig, ALICE_ID).expect("server should spawn Alice");
    let _server_bob = find_server_player(&mut rig, BOB_ID).expect("server should spawn Bob");
    let client0_alice =
        find_predicted_player(&mut rig, 0, ALICE_ID).expect("Alice client should see Alice");
    let client1_alice =
        find_predicted_player(&mut rig, 1, ALICE_ID).expect("Bob client should see Alice");

    let start = server_pos(&rig, server_alice);
    set_client_move(&mut rig, 0, Vec2::Y);
    rig.advance(50);

    let server_after = server_pos(&rig, server_alice);
    assert!(
        server_after.z > start.z + 0.5,
        "server must receive live UDP input from Alice: start={start:?}, after={server_after:?}"
    );
    assert!(
        visible_pos(&rig, 0, client0_alice).z > start.z + 0.5,
        "Alice's visible predicted Transform must move"
    );
    let bob_view = visible_pos(&rig, 1, client1_alice);
    assert!(
        bob_view.z > start.z + 0.5,
        "Bob must see Alice's visible Transform move over real UDP: start={start:?}, bob_view={bob_view:?}"
    );
    assert!(
        bob_view.distance(server_after) <= 0.35,
        "Bob's visible Alice should track server Alice: bob_view={bob_view:?}, server={server_after:?}"
    );
}
