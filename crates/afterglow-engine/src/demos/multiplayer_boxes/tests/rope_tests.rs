use std::time::Duration;

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;

use super::*;
use crate::{
    core::identity::StableEntityId,
    input::{AfterglowAction, default_gameplay_input_map},
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
    app.init_resource::<PlayerName>()
        .init_resource::<crate::core::identity::StableIdAllocator>()
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

/// Releasing F creates a RopeLink for the nearest box.
#[test]
fn toggle_rope_release_f_ropes_nearest_box() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();
    let (_player, _box_entity) = spawn_player_and_box(&mut app);

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
    app.update();
    assert!(
        rope_link_for_box(&mut app, 0).is_none(),
        "rope link should not be spawned while F is held"
    );

    // Release F (should rope)
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyF);
    app.update();

    assert!(
        rope_link_for_box(&mut app, 0).is_some(),
        "rope link should be spawned after releasing F"
    );
}

/// Releasing F again releases the box.
#[test]
fn toggle_rope_release_f_again_releases_box() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();
    let (_player, _box_entity) = spawn_player_and_box(&mut app);

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
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyF);
    app.update();
    assert!(rope_link_for_box(&mut app, 0).is_some());

    // Wait out the anti-stale-input cooldown, then press/release F again to unrope.
    for _ in 0..12 {
        app.update();
    }
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyF);
    app.update();
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyF);
    app.update();

    assert!(
        rope_link_for_box(&mut app, 0).is_none(),
        "rope link should be released after releasing F again"
    );
}

/// A stale duplicate release immediately after attach must not drop the rope.
#[test]
fn local_duplicate_release_inside_cooldown_does_not_drop_rope() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();
    let (_player, _box_entity) = spawn_player_and_box(&mut app);

    app.add_systems(
        PreUpdate,
        super::super::rope::toggle_rope
            .after(leafwing_input_manager::plugin::InputManagerSystem::Update),
    );

    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyF);
    app.update();
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyF);
    app.update();
    assert!(rope_link_for_box(&mut app, 0).is_some());

    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyF);
    app.update();
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyF);
    app.update();

    assert!(
        rope_link_for_box(&mut app, 0).is_some(),
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

    app.world_mut().spawn((
        KinematicBox {
            initial_pos: Vec3::new(100.0, 0.5, 0.0),
        },
        test_box_id(0),
        Transform::from_xyz(100.0, 0.5, 0.0),
        avian3d::prelude::RigidBody::Dynamic,
        LinearVelocity::ZERO,
    ));

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
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyF);
    app.update();

    assert!(
        rope_link_for_box(&mut app, 0).is_none(),
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
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyF);
    app.update();

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
