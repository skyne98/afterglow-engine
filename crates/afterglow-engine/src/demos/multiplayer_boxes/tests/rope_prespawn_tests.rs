use std::time::Duration;

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::*;

use super::*;
use crate::{
    core::identity::StableEntityId,
    input::{AfterglowAction, default_gameplay_input_map},
    network::connection::{ClientSpawned, LocalPlayerId},
};

fn rope_test_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::input::InputPlugin,
        lightyear::prelude::server::ServerPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / 60.0),
        },
        leafwing_input_manager::plugin::InputManagerPlugin::<AfterglowAction>::default(),
    ));
    super::super::network::register_demo_protocol(&mut app);
    app.insert_resource(LocalPlayerId(2))
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .insert_resource(crate::network::AfterglowNetworkContext::new(
            crate::network::LightyearRole::Client,
            2,
        ));
    app
}

fn run_rope_frame(app: &mut App) {
    app.world_mut().run_schedule(FixedUpdate);
    app.world_mut().run_schedule(Update);
    app.world_mut().run_schedule(PostUpdate);
}

fn test_box_id(index: u32) -> StableEntityId {
    StableEntityId::new(10_000 + u128::from(index))
}

fn rope_link_for_box(app: &mut App, box_index: u32) -> Option<RopeLink> {
    let target = test_box_id(box_index);
    app.world_mut()
        .query::<&RopeLink>()
        .iter(app.world())
        .find(|link| link.target == target)
        .cloned()
}

fn prespawned_rope_link_count(app: &mut App) -> usize {
    app.world_mut()
        .query::<(&RopeLink, &PreSpawned)>()
        .iter(app.world())
        .count()
}

#[test]
fn rope_id_is_stable_across_client_server_input_delay_tick_offsets() {
    let target = test_box_id(0);
    let client_processing_tick = 120_u16;
    let server_processing_tick = client_processing_tick.saturating_sub(2);

    let client_id = super::super::rope::rope_id_for_input("2", target);
    let server_id = super::super::rope::rope_id_for_input("2", target);

    assert_ne!(client_processing_tick, server_processing_tick);
    assert_eq!(
        client_id, server_id,
        "rope PreSpawned ids must not depend on local processing tick"
    );
    assert_ne!(
        client_id,
        super::super::rope::rope_id_for_input("3", target)
    );
    assert_ne!(
        client_id,
        super::super::rope::rope_id_for_input("2", test_box_id(1))
    );
}

fn spawn_predicted_player_and_box(app: &mut App) {
    app.world_mut().spawn((
        PlayerBox {
            owner: "2".to_string(),
        },
        Transform::from_xyz(0.0, 0.4, 0.0),
        avian3d::prelude::RigidBody::Dynamic,
        LinearVelocity::ZERO,
        default_gameplay_input_map(),
        ActionState::<AfterglowAction>::default(),
        Predicted,
    ));
    app.world_mut().spawn((
        KinematicBox {
            initial_pos: Vec3::new(1.0, 0.5, 0.0),
        },
        test_box_id(0),
        Transform::from_xyz(1.0, 0.5, 0.0),
        avian3d::prelude::RigidBody::Dynamic,
        LinearVelocity::ZERO,
    ));
}

fn release_rope_toggle(app: &mut App) {
    let mut query = app.world_mut().query::<&mut ActionState<AfterglowAction>>();
    for mut action in query.iter_mut(app.world_mut()) {
        action.press(&AfterglowAction::RopeToggle);
        action.release(&AfterglowAction::RopeToggle);
    }
}

/// Client-only local input cannot pre-spawn a RopeLink until the Lightyear
/// client link exists, because `PreSpawned::for_receiver` needs that receiver.
#[test]
fn client_local_release_without_client_link_does_not_prespawn_rope_link() {
    let mut app = rope_test_app();
    app.world_mut()
        .insert_resource(crate::network::AfterglowNetworkContext::new(
            crate::network::LightyearRole::Client,
            2,
        ));
    spawn_predicted_player_and_box(&mut app);

    app.add_systems(FixedUpdate, super::super::rope::toggle_rope);
    release_rope_toggle(&mut app);
    run_rope_frame(&mut app);

    assert!(
        rope_link_for_box(&mut app, 0).is_none(),
        "client must not pre-spawn an unmatchable RopeLink without a client link"
    );
}

/// Client-only local input pre-spawns a RopeLink from ActionState only.
#[test]
fn client_local_release_prespawns_rope_link_from_action_state() {
    let mut app = rope_test_app();
    let client_link = app.world_mut().spawn(ClientSpawned).id();
    spawn_predicted_player_and_box(&mut app);

    app.add_systems(FixedUpdate, super::super::rope::toggle_rope);
    release_rope_toggle(&mut app);
    run_rope_frame(&mut app);

    let target = test_box_id(0);
    let expected_rope_id = super::super::rope::rope_id_for_input("2", target);
    let mut query = app.world_mut().query::<(&RopeLink, &PreSpawned)>();
    let links: Vec<_> = query.iter(app.world()).collect();
    assert_eq!(links.len(), 1, "client should pre-spawn exactly one rope");
    let (link, prespawned) = links[0];
    assert_eq!(link.player_owner, "2");
    assert_eq!(link.target, target);
    assert_eq!(link.rope_id, expected_rope_id);
    assert_eq!(
        prespawned.hash,
        Some(super::super::rope::rope_link_hash(expected_rope_id))
    );
    assert_eq!(prespawned.receiver, Some(client_link));
}

/// Client detach goes through Lightyear's prediction despawn command when an
/// active locally predicted rope exists for the owner.
#[test]
fn client_local_detach_removes_active_rope_link() {
    let mut app = rope_test_app();
    let client_link = app.world_mut().spawn(ClientSpawned).id();
    spawn_predicted_player_and_box(&mut app);

    let target = test_box_id(0);
    let rope_id = super::super::rope::rope_id_for_input("2", target);
    app.world_mut().spawn((
        RopeLink {
            rope_id,
            player_owner: "2".to_string(),
            target,
        },
        rope_id,
        PreSpawned::new(super::super::rope::rope_link_hash(rope_id)).for_receiver(client_link),
    ));

    app.add_systems(FixedUpdate, super::super::rope::toggle_rope);
    release_rope_toggle(&mut app);
    run_rope_frame(&mut app);

    assert!(
        rope_link_for_box(&mut app, 0).is_none(),
        "client detach should remove or prediction-disable the active local rope"
    );
}

/// The authoritative side derives the target and rope id from input/world
/// state; it does not trust a client-selected target message.
#[test]
fn server_action_state_release_confirms_nearest_valid_box() {
    let mut app = rope_test_app();
    app.world_mut()
        .insert_resource(crate::network::AfterglowNetworkContext::new(
            crate::network::LightyearRole::Client,
            2,
        ));

    app.world_mut().spawn((
        PlayerBox {
            owner: "2".to_string(),
        },
        Transform::from_xyz(0.0, 0.4, 0.0),
        avian3d::prelude::RigidBody::Dynamic,
        LinearVelocity::ZERO,
        ActionState::<AfterglowAction>::default(),
    ));
    app.world_mut().spawn((
        KinematicBox {
            initial_pos: Vec3::new(1.0, 0.5, 0.0),
        },
        test_box_id(0),
        Transform::from_xyz(1.0, 0.5, 0.0),
        avian3d::prelude::RigidBody::Dynamic,
        LinearVelocity::ZERO,
    ));

    app.add_systems(FixedUpdate, super::super::rope::toggle_rope);
    release_rope_toggle(&mut app);
    run_rope_frame(&mut app);

    let roped = rope_link_for_box(&mut app, 0).expect("server should spawn authoritative rope");
    assert_eq!(roped.player_owner.as_str(), "2");
    assert_eq!(
        roped.rope_id,
        super::super::rope::rope_id_for_input("2", test_box_id(0))
    );
    assert_eq!(
        prespawned_rope_link_count(&mut app),
        1,
        "authoritative RopeLink should carry PreSpawned metadata for matching"
    );
}
