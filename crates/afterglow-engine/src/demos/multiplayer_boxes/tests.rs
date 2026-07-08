mod color_tests;
mod connection_lifecycle_tests;
mod input;
mod movement_unit_tests;
mod physics_runtime_tests;
mod prediction_target_tests;
mod rope_hider_tests;
mod rope_highlight_tests;
mod rope_joint_tests;
mod rope_prespawn_tests;
mod rope_release_tests;
mod rope_tests;

use bevy::prelude::*;
use lightyear::prelude::Predicted;

use crate::{
    core::identity::StableEntityId,
    network::connection::{ConnectionEvent, ConnectionEventKind, LocalPlayerId},
};

use super::{camera::*, movement::*, protocol::*, scene::*};

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, avian3d::prelude::PhysicsPlugins::default()));

    app.insert_resource(LocalPlayerId(2))
        .init_resource::<DemoInput>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();

    app
}

#[test]
fn plugin_builds_and_registers_types() {
    let mut app = test_app();

    let entity = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "test".to_string(),
            },
            KinematicBox {
                initial_pos: Vec3::ZERO,
            },
        ))
        .id();

    assert!(app.world().get::<PlayerBox>(entity).is_some());
    assert!(app.world().get::<KinematicBox>(entity).is_some());
}

#[test]
fn scene_entity_counts_are_correct() {
    let mut app = test_app();
    app.add_systems(Startup, spawn_lights);
    app.add_systems(Startup, |mut commands: Commands| {
        let positions = [
            Vec3::new(-4.0, 0.5, -4.0),
            Vec3::new(4.0, 0.5, -4.0),
            Vec3::new(-4.0, 0.5, 4.0),
            Vec3::new(4.0, 0.5, 4.0),
            Vec3::new(-2.0, 0.5, 0.0),
            Vec3::new(2.0, 0.5, 0.0),
            Vec3::new(0.0, 0.5, -2.0),
            Vec3::new(0.0, 0.5, 2.0),
        ];
        for pos in positions.iter() {
            commands.spawn(KinematicBox { initial_pos: *pos });
        }
    });

    app.finish();
    app.cleanup();
    app.update();

    let world = app.world_mut();
    let kinematic_count = world
        .query_filtered::<Entity, With<KinematicBox>>()
        .iter(world)
        .count();
    let player_count = world
        .query_filtered::<Entity, With<PlayerBox>>()
        .iter(world)
        .count();
    let light_count = world
        .query_filtered::<Entity, With<PointLight>>()
        .iter(world)
        .count();

    assert_eq!(kinematic_count, 8, "should have 8 kinematic boxes");
    assert_eq!(player_count, 0, "no player boxes before spawn");
    assert_eq!(light_count, 1, "should have 1 point light");
}

#[test]
fn client_arena_visuals_have_static_physics_colliders() {
    let mut app = test_app();
    app.add_systems(Startup, spawn_client_arena_visuals);

    app.finish();
    app.cleanup();
    app.update();

    let static_collider_count = app
        .world_mut()
        .query::<(&avian3d::prelude::RigidBody, &avian3d::prelude::Collider)>()
        .iter(app.world())
        .filter(|(body, _)| matches!(**body, avian3d::prelude::RigidBody::Static))
        .count();

    assert_eq!(
        static_collider_count, 5,
        "client prediction should include floor and four wall colliders"
    );
}

#[test]
fn camera_setup_follows_player() {
    let mut app = test_app();
    let _player = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "2".to_string(),
            },
            avian3d::prelude::Position::from(Vec3::new(2.0, 0.5, 3.0)),
            lightyear::prelude::Predicted,
        ))
        .id();

    app.add_systems(
        Update,
        (
            setup_camera.run_if(|cam: Query<&DemoCamera>| cam.is_empty()),
            follow_camera_system,
        ),
    );

    app.update();

    let camera_exists = app
        .world_mut()
        .query_filtered::<Entity, With<DemoCamera>>()
        .iter(app.world())
        .next()
        .is_some();
    assert!(camera_exists, "camera should be spawned");
}

#[test]
fn camera_follows_session_member_owner_when_player_name_differs() {
    let mut app = test_app();
    app.world_mut()
        .insert_resource(crate::network::connection::LocalPlayerId(2));

    app.world_mut().spawn((
        PlayerBox {
            owner: "2".to_string(),
        },
        Transform::from_xyz(3.0, 0.5, 4.0),
        lightyear::prelude::Predicted,
    ));

    app.add_systems(
        Update,
        (
            setup_camera.run_if(|cam: Query<&DemoCamera>| cam.is_empty()),
            follow_camera_system,
        ),
    );
    app.update();

    let camera_translation = app
        .world_mut()
        .query_filtered::<&Transform, With<DemoCamera>>()
        .single(app.world())
        .expect("camera should exist")
        .translation;
    assert_eq!(
        camera_translation,
        Vec3::new(3.0, 0.5, 4.0) + Vec3::new(0.0, 8.0, -6.0)
    );
}

#[test]
fn replicated_boxes_get_client_visuals() {
    let mut app = test_app();
    app.add_systems(
        PreUpdate,
        (
            attach_predicted_player_physics,
            attach_predicted_kinematic_physics,
        ),
    );
    app.add_systems(
        Update,
        (
            attach_replicated_player_visuals,
            attach_replicated_kinematic_visuals,
        ),
    );

    let player = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "alice".to_string(),
            },
            Transform::from_xyz(1.0, 0.4, 0.0),
            lightyear::prelude::Predicted,
        ))
        .id();
    let box_entity = app
        .world_mut()
        .spawn((
            KinematicBox {
                initial_pos: Vec3::new(2.0, 0.5, 0.0),
            },
            StableEntityId::new(10_000),
            Transform::from_xyz(2.0, 0.5, 0.0),
            lightyear::prelude::Predicted,
        ))
        .id();

    app.update();

    for entity in [player, box_entity] {
        assert!(
            app.world().get::<Mesh3d>(entity).is_some(),
            "entity {entity:?} should get a mesh"
        );
        assert!(
            app.world()
                .get::<MeshMaterial3d<StandardMaterial>>(entity)
                .is_some(),
            "entity {entity:?} should get a material"
        );
    }
    assert!(
        app.world()
            .get::<avian3d::prelude::RigidBody>(box_entity)
            .is_some(),
        "predicted cubes should get local physics bodies for immediate contacts"
    );
    assert!(
        app.world()
            .get::<avian3d::prelude::Collider>(box_entity)
            .is_some(),
        "predicted cubes should get local colliders for immediate contacts"
    );
}

#[test]
fn confirmed_local_box_is_not_rendered_before_predicted_copy_exists() {
    let mut app = test_app();
    app.insert_resource(crate::network::AfterglowNetworkContext::new(
        crate::network::LightyearRole::Client,
        2,
    ));
    app.add_systems(Update, attach_replicated_player_visuals);

    let entity = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "2".to_string(),
            },
            Transform::from_xyz(1.0, PLAYER_SIZE, 0.0),
        ))
        .id();

    app.update();

    assert!(app.world().get::<Mesh3d>(entity).is_none());
}

#[test]
fn local_replicated_box_gets_prediction_physics_components() {
    let mut app = test_app();
    app.insert_resource(crate::network::AfterglowNetworkContext::new(
        crate::network::LightyearRole::Client,
        2,
    ));
    app.add_systems(PreUpdate, attach_predicted_player_physics);
    app.add_systems(Update, attach_replicated_player_visuals);

    let entity = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "2".to_string(),
            },
            Transform::from_xyz(1.0, PLAYER_SIZE, 0.0),
            Predicted,
        ))
        .id();

    app.update();

    assert!(app.world().get::<Mesh3d>(entity).is_some());
    assert!(
        app.world()
            .get::<avian3d::prelude::RigidBody>(entity)
            .is_some()
    );
    assert!(
        app.world()
            .get::<avian3d::prelude::LinearVelocity>(entity)
            .is_some(),
        "local predicted box should have velocity for same movement system"
    );
}

#[test]
fn disconnect_despawns_owned_player_and_rope_links() {
    let mut app = test_app();
    app.add_observer(super::server::despawn_player_on_disconnected);

    let player = app
        .world_mut()
        .spawn(PlayerBox {
            owner: "2".to_string(),
        })
        .id();
    let rope = app
        .world_mut()
        .spawn(RopeLink {
            rope_id: StableEntityId::new(20_000),
            player_owner: "2".to_string(),
            target: StableEntityId::new(10_000),
        })
        .id();

    app.world_mut().commands().trigger(ConnectionEvent {
        kind: ConnectionEventKind::Disconnected {
            reason: "test disconnect".to_string(),
        },
        player_id: 2,
        link_entity: Entity::PLACEHOLDER,
    });
    app.update();

    assert!(
        app.world().get::<PlayerBox>(player).is_none(),
        "disconnect should despawn the owned player entity"
    );
    assert!(
        app.world().get::<RopeLink>(rope).is_none(),
        "disconnect should despawn stale ropes owned by that player"
    );
}

#[test]
fn predicted_player_physics_has_locked_axes() {
    let mut app = test_app();
    app.add_systems(PreUpdate, attach_predicted_player_physics);

    let entity = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "locks".to_string(),
            },
            Transform::from_xyz(0.0, 0.4, 0.0),
            lightyear::prelude::Predicted,
        ))
        .id();

    app.update();

    assert!(
        app.world()
            .get::<avian3d::prelude::LockedAxes>(entity)
            .is_some(),
        "predicted player physics should include LockedAxes"
    );
    assert!(
        app.world()
            .get::<avian3d::prelude::LockedAxes>(entity)
            .is_some_and(|l| l.is_rotation_locked()),
        "predicted player LockedAxes should have rotation locked"
    );
}

#[test]
fn predicted_player_physics_preserves_existing_velocity() {
    let mut app = test_app();
    app.add_systems(PreUpdate, attach_predicted_player_physics);

    let expected_velocity = avian3d::prelude::LinearVelocity(Vec3::new(1.0, 0.0, 2.0));
    let entity = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "preserve-velocity".to_string(),
            },
            Transform::from_xyz(0.0, 0.4, 0.0),
            Predicted,
            expected_velocity,
        ))
        .id();

    app.update();

    assert_eq!(
        app.world()
            .get::<avian3d::prelude::LinearVelocity>(entity)
            .copied(),
        Some(expected_velocity),
        "predicted physics attachment must not zero a replicated velocity"
    );
}

#[test]
fn predicted_kinematic_physics_has_locked_axes() {
    let mut app = test_app();
    app.add_systems(PreUpdate, attach_predicted_kinematic_physics);

    let entity = app
        .world_mut()
        .spawn((
            KinematicBox {
                initial_pos: Vec3::new(2.0, 0.5, 0.0),
            },
            Transform::from_xyz(2.0, 0.5, 0.0),
            lightyear::prelude::Predicted,
        ))
        .id();

    app.update();

    assert!(
        app.world()
            .get::<avian3d::prelude::LockedAxes>(entity)
            .is_some(),
        "predicted kinematic physics should include LockedAxes"
    );
    assert!(
        app.world()
            .get::<avian3d::prelude::LockedAxes>(entity)
            .is_some_and(|l| l.is_rotation_locked()),
        "predicted kinematic LockedAxes should have rotation locked"
    );
}

#[test]
fn spawn_player_box_helper_has_locked_axes() {
    let mut app = test_app();
    let entity = spawn_player_box(&mut app.world_mut().commands(), "locked-test", Vec3::ZERO);
    app.update();

    let locked = app
        .world()
        .get::<avian3d::prelude::LockedAxes>(entity)
        .expect("spawn_player_box should include LockedAxes");
    assert!(
        locked.is_rotation_locked(),
        "spawn_player_box LockedAxes should have rotation locked"
    );
}
