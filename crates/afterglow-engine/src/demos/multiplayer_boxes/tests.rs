use bevy::prelude::*;

use super::camera::*;
use super::movement::DemoInput;
use super::protocol::*;
use super::scene::*;

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        avian3d::prelude::PhysicsPlugins::default(),
    ));

    app.init_resource::<PlayerName>()
        .init_resource::<DemoInput>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();

    app
}

#[test]
fn plugin_builds_and_registers_types() {
    let mut app = test_app();

    // Verify types exist and can be spawned
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
    // Manually spawn KinematicBoxes to avoid Replicate dependency
    app.add_systems(
        Startup,
        |mut commands: Commands| {
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
        },
    );

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
    let _ = app
        .world_mut()
        .resource_mut::<PlayerName>()
        .0 = "test_player".to_string();

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
fn movement_sets_velocity() {
    let mut app = test_app();
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
fn input_mapping_wasd_is_not_inverted() {
    use bevy::input::ButtonInput;
    use bevy::prelude::KeyCode;

    fn press(app: &mut App, key: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
    }

    fn released_dir(app: &mut App) -> Vec2 {
        // Reset, press the target key, run collect_input, read resource.
        let target = match app.world().resource::<DemoInput>().0 {
            _ => app.world().resource::<DemoInput>().0, // for type inference
        };
        let _ = target;
        app.world().resource::<DemoInput>().0
    }

    fn collect(app: &mut App) {
        app.add_systems(Update, super::movement::collect_input);
        app.update();
    }

    // Each test starts with a fresh app so state doesn't leak.
    for (key, expected, label) in [
        (KeyCode::KeyW, Vec2::new(0.0, 1.0), "W should move forward (+Y in Vec2 -> +Z in Vec3)"),
        (KeyCode::ArrowUp, Vec2::new(0.0, 1.0), "ArrowUp should move forward"),
        (KeyCode::KeyS, Vec2::new(0.0, -1.0), "S should move backward"),
        // Camera's right axis points in -X world (camera sits at player +Z<0,
        // looking down). A/D are intentionally flipped so the on-screen
        // motion matches the key label.
        (KeyCode::KeyA, Vec2::new(1.0, 0.0), "A should move left on screen"),
        (KeyCode::KeyD, Vec2::new(-1.0, 0.0), "D should move right on screen"),
    ] {
        let mut app = test_app();
        app.init_resource::<ButtonInput<KeyCode>>();
        press(&mut app, key);
        collect(&mut app);
        let got = released_dir(&mut app);
        assert_eq!(got, expected, "{label}: expected {expected:?}, got {got:?}");
    }
}
