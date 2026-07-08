use std::time::Duration;

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;

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
        .init_resource::<crate::core::identity::StableIdAllocator>()
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

fn release_rope_toggle(app: &mut App) {
    let mut world = app.world_mut();
    let mut query = world.query::<&mut ActionState<AfterglowAction>>();
    for mut action in query.iter_mut(&mut world) {
        action.press(&AfterglowAction::RopeToggle);
        action.release(&AfterglowAction::RopeToggle);
    }
}

/// Helper: spawn a player (with InputMap + ActionState) and a nearby box.
fn spawn_player_and_box(app: &mut App) -> (Entity, Entity) {
    let player = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "alice".to_string(),
            },
            Transform::from_xyz(0.0, 0.4, 0.0),
            avian3d::prelude::RigidBody::Dynamic,
            LinearVelocity::ZERO,
            default_gameplay_input_map(),
            ActionState::<AfterglowAction>::default(),
        ))
        .id();

    let meshes = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::from_size(Vec3::splat(1.0)));
    let material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());

    let box_entity = app
        .world_mut()
        .spawn((
            KinematicBox {
                initial_pos: Vec3::new(1.0, 0.5, 0.0),
            },
            test_box_id(0),
            Transform::from_xyz(1.0, 0.5, 0.0),
            avian3d::prelude::RigidBody::Dynamic,
            LinearVelocity::ZERO,
            Mesh3d(meshes),
            MeshMaterial3d(material),
            BoxMaterial { base_hue: 0.0 },
        ))
        .id();

    (player, box_entity)
}

fn test_box_id(index: u32) -> StableEntityId {
    StableEntityId::new(10_000 + u128::from(index))
}

fn test_rope_id(index: u32) -> StableEntityId {
    StableEntityId::new(20_000 + u128::from(index))
}

fn rope_link_for_box(app: &mut App, box_index: u32) -> Option<RopeLink> {
    let target = test_box_id(box_index);
    app.world_mut()
        .query::<&RopeLink>()
        .iter(app.world())
        .find(|link| link.target == target)
        .cloned()
}

fn rope_link_for_owner(app: &mut App, owner: &str) -> Option<RopeLink> {
    app.world_mut()
        .query::<&RopeLink>()
        .iter(app.world())
        .find(|link| link.player_owner == owner)
        .cloned()
}

// ---------------------------------------------------------------------------
// Rope toggle tests (use ActionState, NOT ButtonInput directly)
// ---------------------------------------------------------------------------

#[test]
fn predicted_rope_attach_uses_horizontal_range_at_spawn_height() {
    let mut app = rope_test_app();
    app.world_mut().spawn(ClientSpawned);
    app.world_mut().spawn((
        PlayerBox {
            owner: "alice".to_string(),
        },
        Transform::from_xyz(5.0, PLAYER_SIZE, 0.0),
        default_gameplay_input_map(),
        ActionState::<AfterglowAction>::default(),
        Predicted,
    ));
    app.world_mut().spawn((
        KinematicBox {
            initial_pos: Vec3::new(2.0, KINEMATIC_BOX_SIZE, 0.0),
        },
        test_box_id(0),
        Transform::from_xyz(2.0, KINEMATIC_BOX_SIZE, 0.0),
    ));
    app.add_systems(FixedUpdate, super::super::rope::toggle_rope);

    release_rope_toggle(&mut app);
    run_rope_frame(&mut app);

    assert!(
        rope_link_for_owner(&mut app, "alice").is_some(),
        "client prediction must allow the spawn-position grab at exact horizontal range"
    );
}

#[test]
fn rope_attach_uses_horizontal_range_at_spawn_height() {
    let mut app = rope_test_app();
    app.world_mut().spawn((
        PlayerBox {
            owner: "alice".to_string(),
        },
        Transform::from_xyz(5.0, PLAYER_SIZE, 0.0),
        ActionState::<AfterglowAction>::default(),
    ));
    app.world_mut().spawn((
        KinematicBox {
            initial_pos: Vec3::new(2.0, KINEMATIC_BOX_SIZE, 0.0),
        },
        test_box_id(0),
        Transform::from_xyz(2.0, KINEMATIC_BOX_SIZE, 0.0),
    ));
    app.add_systems(FixedUpdate, super::super::rope::toggle_rope);

    release_rope_toggle(&mut app);
    run_rope_frame(&mut app);

    assert!(
        rope_link_for_owner(&mut app, "alice").is_some(),
        "player and block centers differ vertically; horizontal spawn-range grab should still attach"
    );
}

#[test]
fn authoritative_rope_attach_allows_small_prediction_delay_slack() {
    let mut app = rope_test_app();
    app.world_mut().spawn((
        PlayerBox {
            owner: "alice".to_string(),
        },
        Transform::from_xyz(4.05, 0.4, 0.0),
        ActionState::<AfterglowAction>::default(),
    ));
    app.world_mut().spawn((
        KinematicBox {
            initial_pos: Vec3::new(1.0, 0.5, 0.0),
        },
        test_box_id(0),
        Transform::from_xyz(1.0, 0.5, 0.0),
    ));
    app.add_systems(FixedUpdate, super::super::rope::toggle_rope);

    release_rope_toggle(&mut app);
    run_rope_frame(&mut app);

    assert!(
        rope_link_for_owner(&mut app, "alice").is_some(),
        "server should confirm a rope when input delay leaves authority only slightly outside the exact client grab range"
    );
}

/// Releasing F creates a RopeLink for the nearest box.
#[test]
fn toggle_rope_release_f_ropes_nearest_box() {
    let mut app = rope_test_app();
    let (_player, _box_entity) = spawn_player_and_box(&mut app);

    app.add_systems(FixedUpdate, super::super::rope::toggle_rope);

    release_rope_toggle(&mut app);
    run_rope_frame(&mut app);

    assert!(
        rope_link_for_box(&mut app, 0).is_some(),
        "rope link should be spawned after releasing F"
    );
}

/// Releasing F again releases the box.
#[test]
fn toggle_rope_release_f_again_releases_box() {
    let mut app = rope_test_app();
    let (_player, _box_entity) = spawn_player_and_box(&mut app);

    app.add_systems(FixedUpdate, super::super::rope::toggle_rope);

    release_rope_toggle(&mut app);
    run_rope_frame(&mut app);
    assert!(rope_link_for_box(&mut app, 0).is_some());

    release_rope_toggle(&mut app);
    run_rope_frame(&mut app);

    assert!(
        rope_link_for_box(&mut app, 0).is_none(),
        "rope link should be released after releasing F again"
    );
}

/// A second release immediately detaches; there is no custom cooldown state.
#[test]
fn second_action_release_detaches_without_custom_cooldown() {
    let mut app = rope_test_app();
    let (_player, _box_entity) = spawn_player_and_box(&mut app);

    app.add_systems(FixedUpdate, super::super::rope::toggle_rope);

    release_rope_toggle(&mut app);
    run_rope_frame(&mut app);
    assert!(rope_link_for_box(&mut app, 0).is_some());

    release_rope_toggle(&mut app);
    run_rope_frame(&mut app);

    assert!(
        rope_link_for_box(&mut app, 0).is_none(),
        "a real second ActionState release should detach immediately"
    );
}

#[test]
fn client_toggle_rope_skips_non_predicted_player_copies() {
    let mut app = rope_test_app();
    app.world_mut().spawn(ClientSpawned);

    app.world_mut().spawn((
        PlayerBox {
            owner: "2".to_string(),
        },
        Transform::from_xyz(0.0, 0.4, 0.0),
        avian3d::prelude::RigidBody::Dynamic,
        LinearVelocity::ZERO,
        default_gameplay_input_map(),
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

    assert!(
        rope_link_for_box(&mut app, 0).is_none(),
        "client must ignore non-predicted presentation/confirmed player copies"
    );
}

/// Box too far away is not roped.
#[test]
fn toggle_rope_box_too_far_not_roped() {
    let mut app = rope_test_app();

    app.world_mut().spawn((
        PlayerBox {
            owner: "alice".to_string(),
        },
        Transform::from_xyz(0.0, 0.4, 0.0),
        avian3d::prelude::RigidBody::Dynamic,
        LinearVelocity::ZERO,
        default_gameplay_input_map(),
        ActionState::<AfterglowAction>::default(),
    ));

    app.world_mut().spawn((
        KinematicBox {
            initial_pos: Vec3::new(100.0, 0.5, 0.0),
        },
        test_box_id(0),
        Transform::from_xyz(100.0, 0.5, 0.0),
        avian3d::prelude::RigidBody::Dynamic,
        LinearVelocity::ZERO,
    ));

    app.add_systems(FixedUpdate, super::super::rope::toggle_rope);

    release_rope_toggle(&mut app);
    run_rope_frame(&mut app);

    assert!(
        rope_link_for_box(&mut app, 0).is_none(),
        "far box should not be roped"
    );
}

/// Already-roped box is released when F is pressed.
#[test]
fn toggle_rope_skips_already_roped_box() {
    let mut app = rope_test_app();

    app.world_mut().spawn((
        PlayerBox {
            owner: "alice".to_string(),
        },
        Transform::from_xyz(0.0, 0.4, 0.0),
        avian3d::prelude::RigidBody::Dynamic,
        LinearVelocity::ZERO,
        default_gameplay_input_map(),
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
    app.world_mut().spawn(RopeLink {
        rope_id: test_rope_id(0),
        player_owner: "alice".to_string(),
        target: test_box_id(0),
    });

    app.world_mut().spawn((
        KinematicBox {
            initial_pos: Vec3::new(2.0, 0.5, 0.0),
        },
        test_box_id(1),
        Transform::from_xyz(2.0, 0.5, 0.0),
        avian3d::prelude::RigidBody::Dynamic,
        LinearVelocity::ZERO,
    ));

    app.add_systems(FixedUpdate, super::super::rope::toggle_rope);

    release_rope_toggle(&mut app);
    run_rope_frame(&mut app);

    assert!(
        rope_link_for_owner(&mut app, "alice").is_none(),
        "existing rope link should be released"
    );
    assert!(
        rope_link_for_box(&mut app, 1).is_none(),
        "box2 should not be roped (toggle releases, doesn't rope)"
    );
}

// ---------------------------------------------------------------------------
