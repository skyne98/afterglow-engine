//! Production-plugin UDP rope attach regression for multiplayer_boxes.

use crate::rig::LightyearTestRig;
use afterglow_engine::{
    core::identity::StableEntityId,
    demos::multiplayer_boxes::{
        client::MultiplayerBoxesClientPlugin,
        protocol::{KINEMATIC_BOX_SIZE, KinematicBox, PLAYER_SIZE, PlayerBox, RopeLink},
        server::MultiplayerBoxesServerPlugin,
    },
    network::LightyearRole,
};
use avian3d::prelude::*;
use bevy::prelude::*;
use lightyear::prelude::{Predicted, PredictionDisable};

const ALICE_ID: u64 = 1;

fn register_live_boxes(app: &mut App, role: LightyearRole) {
    match role {
        LightyearRole::Server => {
            app.add_plugins(MultiplayerBoxesServerPlugin);
        }
        LightyearRole::Client => {
            app.add_plugins(MultiplayerBoxesClientPlugin);
        }
    }
}

fn live_boxes_rig() -> LightyearTestRig {
    LightyearTestRig::new_afterglow_udp(
        2,
        |app| {
            app.add_plugins((
                bevy::asset::AssetPlugin::default(),
                bevy::input::InputPlugin,
                bevy::transform::TransformPlugin,
                bevy::gizmos::GizmoPlugin,
            ));
            app.init_resource::<Assets<Mesh>>();
            app.init_resource::<Assets<StandardMaterial>>();
            app.add_plugins(
                PhysicsPlugins::default()
                    .build()
                    .disable::<PhysicsTransformPlugin>()
                    .disable::<PhysicsInterpolationPlugin>(),
            );
            app.add_plugins(afterglow_lightyear_avian3d::prelude::AfterglowAvianPlugin::default());
            app.insert_resource(Gravity(Vec3::ZERO));
        },
        register_live_boxes,
        0,
    )
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

fn find_server_block_at(rig: &mut LightyearTestRig, pos: Vec3) -> Option<(Entity, StableEntityId)> {
    let world = rig.server_world_mut();
    let mut q = world.query::<(Entity, &KinematicBox, &StableEntityId)>();
    q.iter(world).find_map(|(entity, box_, id)| {
        (box_.initial_pos.distance(pos) <= 0.01).then_some((entity, *id))
    })
}

fn find_predicted_by_id(
    rig: &mut LightyearTestRig,
    client_id: usize,
    sid: StableEntityId,
) -> Option<Entity> {
    let world = rig.client_world_mut(client_id);
    let mut q = world.query_filtered::<(Entity, &StableEntityId), With<Predicted>>();
    q.iter(world)
        .find_map(|(entity, id)| (*id == sid).then_some(entity))
}

fn set_pose(world: &mut World, entity: Entity, pos: Vec3) {
    if let Some(mut transform) = world.get_mut::<Transform>(entity) {
        transform.translation = pos;
    }
    if let Some(mut position) = world.get_mut::<Position>(entity) {
        position.0 = pos;
    }
    if let Some(mut velocity) = world.get_mut::<LinearVelocity>(entity) {
        velocity.0 = Vec3::ZERO;
    }
}

fn active_rope_count(world: &mut World, owner: u64) -> usize {
    let owner = owner.to_string();
    world
        .query::<(&RopeLink, Option<&PredictionDisable>)>()
        .iter(world)
        .filter(|(link, disabled)| link.player_owner == owner && disabled.is_none())
        .count()
}

fn tap_rope_toggle(rig: &mut LightyearTestRig) {
    rig.client_world_mut(0)
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyF);
    rig.advance(2);
    rig.client_world_mut(0)
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyF);
    rig.advance(1);
}

#[test]
fn udp_live_multiplayer_boxes_spawn_position_rope_attach_replicates() {
    let mut rig = live_boxes_rig();
    rig.connect();
    rig.advance(120);

    tap_rope_toggle(&mut rig);
    rig.advance(80);

    assert_eq!(
        active_rope_count(rig.server_world_mut(), ALICE_ID),
        1,
        "server should confirm Alice's spawn-position rope attach"
    );
    assert_eq!(active_rope_count(rig.client_world_mut(0), ALICE_ID), 1);
    assert_eq!(
        active_rope_count(rig.client_world_mut(1), ALICE_ID),
        1,
        "Bob should see Alice's spawn-position rope attach"
    );
}

#[test]
fn udp_live_multiplayer_boxes_authoritative_rope_replicates_to_other_client() {
    let mut rig = live_boxes_rig();
    rig.connect();
    rig.advance(120);

    let block_pos = Vec3::new(2.0, KINEMATIC_BOX_SIZE, 0.0);
    let (_, block_id) = find_server_block_at(&mut rig, block_pos).expect("server arena block");
    let server_alice = find_server_player(&mut rig, ALICE_ID).expect("server Alice");
    let client0_alice = find_predicted_player(&mut rig, 0, ALICE_ID).expect("client Alice");
    let client0_block = find_predicted_by_id(&mut rig, 0, block_id).expect("client block");

    let hook_pos = Vec3::new(1.55, PLAYER_SIZE, 0.0);
    set_pose(rig.server_world_mut(), server_alice, hook_pos);
    set_pose(rig.client_world_mut(0), client0_alice, hook_pos);
    set_pose(rig.client_world_mut(0), client0_block, block_pos);
    rig.advance(10);

    tap_rope_toggle(&mut rig);
    rig.advance(80);

    assert_eq!(active_rope_count(rig.server_world_mut(), ALICE_ID), 1);
    assert_eq!(active_rope_count(rig.client_world_mut(0), ALICE_ID), 1);
    assert_eq!(
        active_rope_count(rig.client_world_mut(1), ALICE_ID),
        1,
        "the non-owning client must see the server-confirmed rope"
    );
}
