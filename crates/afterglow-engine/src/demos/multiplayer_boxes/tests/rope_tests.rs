use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;

use super::*;
use crate::input::{AfterglowAction, default_gameplay_input_map};

fn rope_test_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        avian3d::prelude::PhysicsPlugins::default(),
        bevy::input::InputPlugin,
        leafwing_input_manager::plugin::InputManagerPlugin::<AfterglowAction>::default(),
    ));
    app.init_resource::<PlayerName>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .insert_resource(crate::network::AfterglowNetworkContext::from_status(
            crate::network::AfterglowConnectionStatus {
                role: crate::network::LightyearRole::Host,
                ..Default::default()
            },
        ));
    app
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
                id: 0,
                initial_pos: Vec3::new(1.0, 0.5, 0.0),
            },
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

// ---------------------------------------------------------------------------
// Rope toggle tests (use ActionState, NOT ButtonInput directly)
// ---------------------------------------------------------------------------

/// Releasing F toggles RopedTo on the nearest box.
#[test]
fn toggle_rope_release_f_ropes_nearest_box() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();
    let (_player, box_entity) = spawn_player_and_box(&mut app);

    app.add_systems(
        PreUpdate,
        super::super::rope::toggle_rope
            .after(leafwing_input_manager::plugin::InputManagerSystem::Update),
    );

    // Press F (not roped yet — toggle is on release)
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyF);
    app.update();
    assert!(
        app.world().get::<RopedTo>(box_entity).is_none(),
        "box should not be roped while F is held"
    );

    // Release F (should rope)
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyF);
    app.update();

    assert!(
        app.world().get::<RopedTo>(box_entity).is_some(),
        "box should be roped after releasing F"
    );
}

/// Releasing F again releases the box.
#[test]
fn toggle_rope_release_f_again_releases_box() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();
    let (_player, box_entity) = spawn_player_and_box(&mut app);

    app.add_systems(
        PreUpdate,
        super::super::rope::toggle_rope
            .after(leafwing_input_manager::plugin::InputManagerSystem::Update),
    );

    // Press then release F to rope
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyF);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyF);
    app.update();
    assert!(app.world().get::<RopedTo>(box_entity).is_some());

    // Wait out the anti-stale-input cooldown, then press/release F again to unrope.
    for _ in 0..8 {
        app.update();
    }
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyF);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyF);
    app.update();

    assert!(
        app.world().get::<RopedTo>(box_entity).is_none(),
        "box should be released after releasing F again"
    );
}

/// The authoritative host consumes a remote client's replicated ActionState
/// release edge and creates the server-owned RopedTo component. This prevents
/// the client's predicted rope from being corrected away immediately.
#[test]
fn server_remote_action_state_release_ropes_nearest_box() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();
    app.world_mut()
        .insert_resource(crate::network::AfterglowNetworkContext::from_status(
            crate::network::AfterglowConnectionStatus {
                role: crate::network::LightyearRole::Host,
                local_member_id: crate::network::session::SessionMemberId::new(1),
                ..Default::default()
            },
        ));

    let remote = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "2".to_string(),
            },
            Transform::from_xyz(0.0, 0.4, 0.0),
            avian3d::prelude::RigidBody::Dynamic,
            LinearVelocity::ZERO,
            ActionState::<AfterglowAction>::default(),
        ))
        .id();

    let box_entity = app
        .world_mut()
        .spawn((
            KinematicBox {
                id: 0,
                initial_pos: Vec3::new(1.0, 0.5, 0.0),
            },
            Transform::from_xyz(1.0, 0.5, 0.0),
            avian3d::prelude::RigidBody::Dynamic,
            LinearVelocity::ZERO,
        ))
        .id();

    app.add_systems(
        Update,
        super::super::rope::server_toggle_remote_ropes_from_inputs,
    );

    app.world_mut()
        .get_mut::<ActionState<AfterglowAction>>(remote)
        .unwrap()
        .press(&AfterglowAction::RopeToggle);
    app.update();
    assert!(app.world().get::<RopedTo>(box_entity).is_none());

    app.world_mut()
        .get_mut::<ActionState<AfterglowAction>>(remote)
        .unwrap()
        .release(&AfterglowAction::RopeToggle);
    app.update();

    let roped = app.world().get::<RopedTo>(box_entity);
    assert_eq!(roped.map(|r| r.player_owner.as_str()), Some("2"));

    // The server may observe stale/replayed states immediately after one
    // release. A false->true->false replay inside the cooldown must not toggle
    // the rope back off.
    app.world_mut()
        .get_mut::<ActionState<AfterglowAction>>(remote)
        .unwrap()
        .press(&AfterglowAction::RopeToggle);
    app.update();
    app.world_mut()
        .get_mut::<ActionState<AfterglowAction>>(remote)
        .unwrap()
        .release(&AfterglowAction::RopeToggle);
    app.update();
    assert_eq!(
        app.world()
            .get::<RopedTo>(box_entity)
            .map(|r| r.player_owner.as_str()),
        Some("2")
    );
}

/// A stale duplicate release immediately after attach must not drop the rope.
#[test]
fn local_duplicate_release_inside_cooldown_does_not_drop_rope() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();
    let (_player, box_entity) = spawn_player_and_box(&mut app);

    app.add_systems(
        PreUpdate,
        super::super::rope::toggle_rope
            .after(leafwing_input_manager::plugin::InputManagerSystem::Update),
    );

    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyF);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyF);
    app.update();
    assert!(app.world().get::<RopedTo>(box_entity).is_some());

    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyF);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyF);
    app.update();

    assert!(
        app.world().get::<RopedTo>(box_entity).is_some(),
        "duplicate/stale release inside cooldown must not immediately drop the rope"
    );
}

/// Box too far away is not roped.
#[test]
fn toggle_rope_box_too_far_not_roped() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();

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

    let box_entity = app
        .world_mut()
        .spawn((
            KinematicBox {
                id: 0,
                initial_pos: Vec3::new(100.0, 0.5, 0.0),
            },
            Transform::from_xyz(100.0, 0.5, 0.0),
            avian3d::prelude::RigidBody::Dynamic,
            LinearVelocity::ZERO,
        ))
        .id();

    app.add_systems(
        PreUpdate,
        super::super::rope::toggle_rope
            .after(leafwing_input_manager::plugin::InputManagerSystem::Update),
    );

    // Press then release F
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyF);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyF);
    app.update();

    assert!(
        app.world().get::<RopedTo>(box_entity).is_none(),
        "far box should not be roped"
    );
}

/// Already-roped box is released when F is pressed.
#[test]
fn toggle_rope_skips_already_roped_box() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();

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

    let box1 = app
        .world_mut()
        .spawn((
            KinematicBox {
                id: 0,
                initial_pos: Vec3::new(1.0, 0.5, 0.0),
            },
            Transform::from_xyz(1.0, 0.5, 0.0),
            avian3d::prelude::RigidBody::Dynamic,
            LinearVelocity::ZERO,
            RopedTo {
                player_owner: "alice".to_string(),
            },
        ))
        .id();

    let box2 = app
        .world_mut()
        .spawn((
            KinematicBox {
                id: 1,
                initial_pos: Vec3::new(2.0, 0.5, 0.0),
            },
            Transform::from_xyz(2.0, 0.5, 0.0),
            avian3d::prelude::RigidBody::Dynamic,
            LinearVelocity::ZERO,
        ))
        .id();

    app.add_systems(
        PreUpdate,
        super::super::rope::toggle_rope
            .after(leafwing_input_manager::plugin::InputManagerSystem::Update),
    );

    // Press then release F
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyF);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyF);
    app.update();

    assert!(
        app.world().get::<RopedTo>(box1).is_none(),
        "box1 should be released"
    );
    assert!(
        app.world().get::<RopedTo>(box2).is_none(),
        "box2 should not be roped (toggle releases, doesn't rope)"
    );
}

// ---------------------------------------------------------------------------
