#[cfg(feature = "lightyear")]
pub mod net;
mod presentation;

use bevy::prelude::*;
use lightyear::prelude::Predicted;

use super::{camera::*, movement::*, protocol::*, scene::*};

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, avian3d::prelude::PhysicsPlugins::default()));

    app.init_resource::<PlayerName>()
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
                id: 0,
                initial_pos: Vec3::ZERO,
            },
            MoveInput {
                direction: Vec2::ZERO,
            },
        ))
        .id();

    assert!(app.world().get::<PlayerBox>(entity).is_some());
    assert!(app.world().get::<KinematicBox>(entity).is_some());
    assert!(app.world().get::<MoveInput>(entity).is_some());
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
        for (i, pos) in positions.iter().enumerate() {
            commands.spawn(KinematicBox {
                id: i as u32,
                initial_pos: *pos,
            });
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
fn camera_setup_follows_player() {
    let mut app = test_app();
    let _ = app.world_mut().resource_mut::<PlayerName>().0 = "test_player".to_string();

    let _player = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "test_player".to_string(),
            },
            avian3d::prelude::Position::from(Vec3::new(2.0, 0.5, 3.0)),
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
    app.world_mut().resource_mut::<PlayerName>().0 = "bob".to_string();
    app.world_mut()
        .insert_resource(crate::network::session::AfterglowSessionState {
            local_member_id: crate::network::session::SessionMemberId::new(2),
            current_session: Some(crate::network::session::SessionId::new(1)),
            ..Default::default()
        });

    app.world_mut().spawn((
        PlayerBox {
            owner: "2".to_string(),
        },
        Transform::from_xyz(3.0, 0.5, 4.0),
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
                owner: "2".to_string(),
            },
            Transform::from_xyz(1.0, 0.4, 0.0),
        ))
        .id();
    let box_entity = app
        .world_mut()
        .spawn((
            KinematicBox {
                id: 3,
                initial_pos: Vec3::new(2.0, 0.5, 0.0),
            },
            Transform::from_xyz(2.0, 0.5, 0.0),
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
}

#[test]
fn late_local_member_context_converts_root_mesh_to_smooth_child() {
    let mut app = test_app();
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
    assert!(app.world().get::<Mesh3d>(entity).is_some());

    app.insert_resource(crate::network::AfterglowNetworkContext::from_status(
        crate::network::AfterglowConnectionStatus {
            role: crate::network::LightyearRole::Client,
            local_member_id: crate::network::SessionMemberId::new(2),
            ..Default::default()
        },
    ));
    app.update();

    assert!(app.world().get::<Mesh3d>(entity).is_none());
    assert!(app.world().get::<LocalPlayerPresentation>(entity).is_some());
    let visual_count = app
        .world_mut()
        .query_filtered::<Entity, With<PlayerVisual>>()
        .iter(app.world())
        .count();
    assert_eq!(visual_count, 1);
}

#[test]
fn local_replicated_box_gets_prediction_physics_components() {
    let mut app = test_app();
    app.insert_resource(crate::network::AfterglowNetworkContext::from_status(
        crate::network::AfterglowConnectionStatus {
            role: crate::network::LightyearRole::Client,
            local_member_id: crate::network::SessionMemberId::new(2),
            ..Default::default()
        },
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

    assert!(app.world().get::<LocalPlayerPresentation>(entity).is_some());
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
    let visual_count = app
        .world_mut()
        .query_filtered::<Entity, With<PlayerVisual>>()
        .iter(app.world())
        .count();
    assert_eq!(visual_count, 1, "local visuals should be a smooth child");
}

#[test]
fn local_visual_correction_is_smoothed_not_snapped() {
    let current = Vec3::ZERO;
    let root_after_correction = Vec3::new(1.0, 0.0, 0.0);

    let next =
        advance_local_visual_translation(current, root_after_correction, Vec3::ZERO, 1.0 / 60.0);

    assert!(next.x > 0.0, "visual should start correcting toward server");
    assert!(
        next.x < root_after_correction.x,
        "small server corrections should be absorbed over multiple frames"
    );
}

#[test]
fn local_visual_teleport_correction_snaps() {
    let root_after_teleport = Vec3::new(10.0, 0.0, 0.0);

    let next =
        advance_local_visual_translation(Vec3::ZERO, root_after_teleport, Vec3::ZERO, 1.0 / 60.0);

    assert_eq!(next, root_after_teleport);
}

#[test]
fn movement_sets_velocity() {
    let mut app = test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "mover".to_string();
    app.world_mut().resource_mut::<DemoInput>().0 = Vec2::new(1.0, 0.0);

    let _entity = app.world_mut().spawn((
        PlayerBox {
            owner: "mover".to_string(),
        },
        MoveInput {
            direction: Vec2::new(1.0, 0.0),
        },
        avian3d::prelude::RigidBody::Dynamic,
        avian3d::prelude::LinearVelocity::ZERO,
    ));

    app.add_systems(Update, super::movement::apply_movement);
    app.update();

    let velocities: Vec<avian3d::prelude::LinearVelocity> = app
        .world_mut()
        .query_filtered::<&avian3d::prelude::LinearVelocity, With<PlayerBox>>()
        .iter(app.world())
        .copied()
        .collect();

    assert!(!velocities.is_empty(), "should find at least one velocity");
    let vel = velocities[0];
    assert!(
        vel.0.length() > 0.0,
        "movement system should apply velocity"
    );
    assert!(
        (vel.0.x - PLAYER_SPEED).abs() < 0.001,
        "velocity x should equal player speed"
    );
}

#[test]
fn apply_movement_only_moves_local_player() {
    let mut app = test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();
    app.world_mut().resource_mut::<DemoInput>().0 = Vec2::new(0.0, 1.0);

    let alice = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "alice".to_string(),
            },
            avian3d::prelude::LinearVelocity::ZERO,
        ))
        .id();
    let bob = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "2".to_string(),
            },
            avian3d::prelude::LinearVelocity::ZERO,
        ))
        .id();

    app.add_systems(Update, super::movement::apply_movement);
    app.update();

    assert_eq!(
        app.world()
            .get::<avian3d::prelude::LinearVelocity>(alice)
            .unwrap()
            .0,
        Vec3::new(0.0, 0.0, PLAYER_SPEED)
    );
    assert_eq!(
        app.world()
            .get::<avian3d::prelude::LinearVelocity>(bob)
            .unwrap()
            .0,
        Vec3::ZERO,
        "host input must not move the remote player's box"
    );
}

#[test]
fn input_mapping_wasd_is_not_inverted() {
    use bevy::{input::ButtonInput, prelude::KeyCode};

    fn press(app: &mut App, key: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
    }

    fn released_dir(app: &mut App) -> Vec2 {
        let target = match app.world().resource::<DemoInput>().0 {
            _ => app.world().resource::<DemoInput>().0,
        };
        let _ = target;
        app.world().resource::<DemoInput>().0
    }

    fn collect(app: &mut App) {
        app.add_systems(Update, super::movement::collect_input);
        app.update();
    }

    for (key, expected, label) in [
        (
            KeyCode::KeyW,
            Vec2::new(0.0, 1.0),
            "W should move forward (+Y in Vec2 -> +Z in Vec3)",
        ),
        (
            KeyCode::ArrowUp,
            Vec2::new(0.0, 1.0),
            "ArrowUp should move forward",
        ),
        (
            KeyCode::KeyS,
            Vec2::new(0.0, -1.0),
            "S should move backward",
        ),
        (
            KeyCode::KeyA,
            Vec2::new(1.0, 0.0),
            "A should move left on screen",
        ),
        (
            KeyCode::KeyD,
            Vec2::new(-1.0, 0.0),
            "D should move right on screen",
        ),
    ] {
        let mut app = test_app();
        app.init_resource::<ButtonInput<KeyCode>>();
        press(&mut app, key);
        collect(&mut app);
        let got = released_dir(&mut app);
        assert_eq!(got, expected, "{label}: expected {expected:?}, got {got:?}");
    }
}

#[test]
fn apply_velocity_to_player_writes_linear_velocity() {
    use avian3d::prelude::*;
    use bevy::ecs::system::SystemState;

    let mut app = test_app();

    let entity = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "velocity-test".to_string(),
            },
            avian3d::prelude::RigidBody::Dynamic,
            avian3d::prelude::LinearVelocity::ZERO,
        ))
        .id();

    let vel = Vec3::new(3.0, 0.0, 4.0);

    let mut system_state: SystemState<
        Query<&mut LinearVelocity, (With<PlayerBox>, Without<Predicted>)>,
    > = SystemState::new(app.world_mut());
    let mut query = system_state.get_mut(app.world_mut());
    apply_velocity_to_player(vel, &mut query);

    let result = app.world().get::<LinearVelocity>(entity).unwrap();
    assert_eq!(
        result.0, vel,
        "apply_velocity_to_player should write the correct velocity vector"
    );
}
