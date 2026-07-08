use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::Predicted;

use super::*;

#[test]
fn movement_sets_velocity() {
    let mut app = test_app();
    let mut action_state = ActionState::<crate::input::AfterglowAction>::default();
    action_state.set_axis_pair(&crate::input::AfterglowAction::Move, Vec2::new(1.0, 0.0));

    let _entity = app.world_mut().spawn((
        PlayerBox {
            owner: "mover".to_string(),
        },
        avian3d::prelude::RigidBody::Dynamic,
        avian3d::prelude::LinearVelocity::ZERO,
        action_state,
    ));

    app.add_systems(Update, apply_movement);
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
        (vel.0.x - (-PLAYER_SPEED)).abs() < 0.001,
        "velocity x should equal -player speed (axis flip)"
    );
}

#[test]
fn apply_movement_only_moves_local_player() {
    let mut app = test_app();
    let mut alice_action = ActionState::<crate::input::AfterglowAction>::default();
    alice_action.set_axis_pair(&crate::input::AfterglowAction::Move, Vec2::new(0.0, 1.0));

    let alice = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "alice".to_string(),
            },
            avian3d::prelude::LinearVelocity::ZERO,
            alice_action,
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

    app.add_systems(Update, apply_movement);
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
        app.world().resource::<DemoInput>().0
    }

    fn collect(app: &mut App) {
        app.add_systems(Update, collect_input);
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
fn player_box_starts_with_zero_velocity() {
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

    let result = app
        .world()
        .get::<avian3d::prelude::LinearVelocity>(entity)
        .unwrap();
    assert_eq!(
        result.0,
        Vec3::ZERO,
        "freshly spawned PlayerBox should have zero velocity"
    );
}

#[test]
fn movement_preserves_existing_transform_rotation() {
    let mut app = test_app();
    let mut action_state = ActionState::<crate::input::AfterglowAction>::default();
    action_state.set_axis_pair(&crate::input::AfterglowAction::Move, Vec2::new(0.0, 1.0));

    let rotation = Quat::from_rotation_y(1.23);
    let entity = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "rot-test".to_string(),
            },
            avian3d::prelude::RigidBody::Dynamic,
            avian3d::prelude::LinearVelocity::ZERO,
            Transform {
                rotation,
                ..default()
            },
            Predicted,
            action_state,
        ))
        .id();

    app.add_systems(Update, apply_predicted_movement);
    app.update();

    let velocity = app
        .world()
        .get::<avian3d::prelude::LinearVelocity>(entity)
        .unwrap()
        .0;
    assert_ne!(velocity, Vec3::ZERO, "test must exercise movement");
    let transform = app.world().get::<Transform>(entity).unwrap();
    assert!(
        (transform.rotation.angle_between(rotation)).abs() < 0.001,
        "apply_movement must not snap Transform.rotation: expected {rotation:?}, got {:?}",
        transform.rotation
    );
}

#[test]
fn movement_preserves_rotation_when_moving_with_linear_velocity_only() {
    let mut app = test_app();
    let mut action_state = ActionState::<crate::input::AfterglowAction>::default();
    action_state.set_axis_pair(&crate::input::AfterglowAction::Move, Vec2::new(1.0, 0.0));

    let rotation = Quat::from_rotation_y(2.71);
    let entity = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "rot-test-2".to_string(),
            },
            avian3d::prelude::RigidBody::Dynamic,
            avian3d::prelude::LinearVelocity::ZERO,
            Transform {
                rotation,
                ..default()
            },
            Predicted,
            action_state,
        ))
        .id();

    app.add_systems(Update, apply_predicted_movement);
    app.update();

    let velocity = app
        .world()
        .get::<avian3d::prelude::LinearVelocity>(entity)
        .unwrap()
        .0;
    assert_ne!(velocity, Vec3::ZERO, "test must exercise movement");
    let transform = app.world().get::<Transform>(entity).unwrap();
    assert_eq!(
        transform.rotation, rotation,
        "movement with LinearVelocity must not touch Transform.rotation"
    );
}
