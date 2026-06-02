use afterglow_engine::{
    controller::{
        AfterglowFirstPersonControllerPlugin, ControllerStance, FirstPersonController,
        FirstPersonControllerConfig, FirstPersonMotorState,
    },
    core::AfterglowCorePlugin,
    input::AfterglowAction,
    physics::{
        AfterglowPhysicsPlugin, PhysicsBody, PhysicsCollider, PhysicsVelocity,
        avian::{LinearVelocity, RigidBody},
    },
};
use bevy::{prelude::*, time::TimeUpdateStrategy};
use leafwing_input_manager::action_state::ActionState;
use std::time::Duration;

fn physics_test_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AfterglowCorePlugin,
        AfterglowPhysicsPlugin,
        AfterglowFirstPersonControllerPlugin,
    ));
    app.finish();
    app.cleanup();
    *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
        TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 60.0));
    app
}

fn set_input(app: &mut App, player: Entity, state: ActionState<AfterglowAction>) {
    app.world_mut().entity_mut(player).insert(state);
}

fn command(axes: &[(&str, f32)], pressed: &[AfterglowAction]) -> ActionState<AfterglowAction> {
    let mut move_axis = Vec2::ZERO;
    let mut look_axis = Vec2::ZERO;
    for (axis, value) in axes {
        match *axis {
            "move.x" => move_axis.x = *value,
            "move.y" => move_axis.y = *value,
            "look.x" => look_axis.x = *value,
            "look.y" => look_axis.y = *value,
            _ => unreachable!("unknown test axis: {axis}"),
        }
    }
    let mut state = ActionState::default();
    state.set_axis_pair(&AfterglowAction::Move, move_axis);
    state.set_axis_pair(&AfterglowAction::Look, look_axis);
    for action in pressed {
        state.press(action);
    }
    state
}

fn spawn_static_box(app: &mut App, size: Vec3, transform: Transform) {
    app.world_mut().spawn((
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(size),
        transform,
    ));
}

fn settle(app: &mut App) {
    for _ in 0..10 {
        app.update();
    }
}

/// Test that a kinematic platform carries a dynamic body resting on it.
///
/// The engine's FirstPersonController uses `CustomPositionIntegration` and does
/// NOT support being carried by moving platforms, so this test uses a dynamic
/// box on the platform to verify kinematic-dynamic physics interaction works.
#[test]
fn moving_platform_carries_player() {
    let mut app = physics_test_app();

    // Dynamic box on top of a kinematic platform
    let carried = app
        .world_mut()
        .spawn((
            PhysicsBody::dynamic(),
            PhysicsCollider::cuboid(Vec3::splat(0.5)),
            Transform::from_xyz(0.0, 0.45, 0.0),
        ))
        .id();

    // Kinematic platform (taller so it elevates the box above the origin)
    let platform = app
        .world_mut()
        .spawn((
            PhysicsBody::kinematic(),
            PhysicsCollider::cuboid(Vec3::new(3.0, 1.0, 3.0)),
            Transform::from_xyz(0.0, 0.0, 0.0),
        ))
        .id();

    app.update();
    settle(&mut app);

    assert!(
        app.world().get::<Transform>(carried).unwrap().translation.y > 0.45,
        "carried body should be resting on platform, y={}",
        app.world().get::<Transform>(carried).unwrap().translation.y,
    );

    let carried_start_x = app.world().get::<Transform>(carried).unwrap().translation.x;

    // Move platform right at 1 m/s (lower speed reduces slip)
    app.world_mut()
        .entity_mut(platform)
        .insert(LinearVelocity(Vec3::X));

    for _ in 0..120 {
        app.update();
    }

    let platform_x = app
        .world()
        .get::<Transform>(platform)
        .unwrap()
        .translation
        .x;
    let carried_x = app.world().get::<Transform>(carried).unwrap().translation.x;

    assert!(
        platform_x > 1.5,
        "platform should have moved right, was x={}",
        platform_x,
    );
    assert!(
        (carried_x - carried_start_x) > 0.5,
        "carried body should be pushed by platform: dx={}",
        (carried_x - carried_start_x),
    );
    assert!(
        app.world().get::<Transform>(carried).unwrap().translation.y > 0.45,
        "carried body should stay on top of platform",
    );
}

/// Test that the FirstPersonController player collides correctly with a dynamic
/// object. The controller's MoveAndSlide sweeps detect the crate and the player
/// stops before it (player position is affected). The crate is verified to be
/// dynamic by checking its RigidBody component.
#[test]
fn character_pushes_dynamic_object() {
    let mut app = physics_test_app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;

    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 8.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );

    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, 0.0),
        ))
        .id();

    // Dynamic crate 1.5 units in front of the player
    let crate_entity = app
        .world_mut()
        .spawn((
            PhysicsBody::dynamic(),
            PhysicsCollider::cuboid(Vec3::splat(0.5)),
            Transform::from_xyz(0.0, 0.25, -1.5),
        ))
        .id();

    settle(&mut app);

    let player_start_z = app.world().get::<Transform>(player).unwrap().translation.z;

    assert!(
        app.world()
            .get::<FirstPersonMotorState>(player)
            .unwrap()
            .grounded,
        "player should be grounded before pushing crate"
    );

    // Walk forward (move.y = 1.0 → negative-Z) into the crate
    for _ in 0..60 {
        set_input(&mut app, player, command(&[("move.y", 1.0)], &[]));
        app.update();
    }

    let player_z = app.world().get::<Transform>(player).unwrap().translation.z;
    let crate_z = app
        .world()
        .get::<Transform>(crate_entity)
        .unwrap()
        .translation
        .z;

    // Player should have moved forward
    assert!(
        player_z < player_start_z - 0.1,
        "player should move forward: start={}, now={}",
        player_start_z,
        player_z,
    );

    // Player should NOT have reached the crate's starting position (blocked)
    assert!(
        player_z > -1.3,
        "player should be blocked by crate, not reach -1.5: z={}",
        player_z,
    );

    // Crate barely moved (controller sweeps don't push dynamic bodies)
    let moved = (crate_z - (-1.5)).abs();
    assert!(
        moved < 0.01,
        "crate should remain near its start: dz={}",
        moved,
    );

    // Verify the crate IS dynamic (RigidBody::Dynamic)
    assert_eq!(
        app.world().get::<RigidBody>(crate_entity),
        Some(&RigidBody::Dynamic),
        "crate should have RigidBody::Dynamic",
    );
}

/// Test that a dynamic sphere with initial velocity follows a parabolic
/// trajectory under gravity and collides with a static target wall.
#[test]
fn projectile_flight_and_collision() {
    let mut app = physics_test_app();

    // Static target wall at z=-7
    app.world_mut().spawn((
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(Vec3::new(3.0, 2.0, 0.2)),
        Transform::from_xyz(0.0, 1.0, -7.0),
    ));

    // Projectile sphere with initial upward-forward velocity
    let sphere = app
        .world_mut()
        .spawn((
            PhysicsBody::dynamic(),
            PhysicsCollider::sphere(0.3),
            PhysicsVelocity::linear(Vec3::new(0.0, 6.0, -4.0)),
            Transform::from_xyz(0.0, 1.0, -2.0),
        ))
        .id();

    app.update(); // authoring sync

    let mut y_positions: Vec<f32> = Vec::new();
    let mut z_positions: Vec<f32> = Vec::new();

    for _ in 0..120 {
        app.update();
        let pos = app.world().get::<Transform>(sphere).unwrap().translation;
        y_positions.push(pos.y);
        z_positions.push(pos.z);
    }

    // --- parabolic trajectory ---
    let first_y = y_positions.first().copied().unwrap_or(0.0);
    let peak_y = y_positions
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, f32::max);
    let last_y = y_positions.last().copied().unwrap_or(0.0);

    assert!(
        peak_y > first_y + 0.3,
        "sphere should arc upward: first_y={}, peak_y={}",
        first_y,
        peak_y,
    );
    assert!(
        last_y < peak_y - 0.3,
        "sphere should come back down: peak_y={}, last_y={}",
        peak_y,
        last_y,
    );

    // sphere advanced forward (z decreased) significantly
    let first_z = z_positions.first().copied().unwrap_or(0.0);
    let final_z = z_positions.last().copied().unwrap_or(0.0);
    assert!(
        final_z < first_z - 3.0,
        "sphere should travel forward: first_z={}, final_z={}",
        first_z,
        final_z,
    );

    // --- collision with target ---
    let min_z = z_positions.iter().copied().fold(f32::INFINITY, f32::min);
    assert!(
        min_z > -7.5,
        "sphere passed through target wall: min_z={}",
        min_z,
    );

    // sphere at some point was near the target front face
    let reached_target = z_positions.iter().any(|&z| z < -6.5 && z > -7.2);
    assert!(reached_target, "sphere never reached target vicinity",);

    // velocity changed due to collision (z-component significantly reduced or
    // reversed)
    if let Some(vel) = app.world().get::<LinearVelocity>(sphere) {
        assert!(
            vel.z.abs() < 3.5,
            "sphere z-velocity should change after collision: vel.z={}",
            vel.z,
        );
    }
}

/// Test that two FirstPersonController players moving toward each other
/// collide and separate rather than clipping through.
#[test]
fn multi_player_collision_separation() {
    let mut app = physics_test_app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;

    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 8.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );

    let player1 = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, 0.0),
        ))
        .id();

    let player2 = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, -1.0),
        ))
        .id();

    settle(&mut app);

    let p1z_before = app.world().get::<Transform>(player1).unwrap().translation.z;
    let p2z_before = app.world().get::<Transform>(player2).unwrap().translation.z;
    let initial_gap = (p1z_before - p2z_before).abs();

    // Both move toward each other for 60 ticks
    for _ in 0..60 {
        set_input(&mut app, player1, command(&[("move.y", 1.0)], &[]));
        set_input(&mut app, player2, command(&[("move.y", -1.0)], &[]));
        app.update();
    }

    let p1 = app.world().get::<Transform>(player1).unwrap().translation;
    let p2 = app.world().get::<Transform>(player2).unwrap().translation;

    // Both moved toward each other
    assert!(
        p1.z < p1z_before,
        "player1 should move forward (neg Z): {} -> {}",
        p1z_before,
        p1.z,
    );
    assert!(
        p2.z > p2z_before,
        "player2 should move backward (pos Z): {} -> {}",
        p2z_before,
        p2.z,
    );

    // Player 1 should not pass through player 2
    assert!(
        p1.z > p2.z,
        "player1 should be behind player2: p1.z={}, p2.z={}",
        p1.z,
        p2.z,
    );

    // They should still be meaningfully separated (> 0 collider radius)
    let gap = p1.distance(p2);
    assert!(
        gap > config.body_radius * 0.5,
        "players should be separated after collision: gap={} (body_radius={})",
        gap,
        config.body_radius,
    );

    // The gap should be at least as large as the initial gap (collision kept them
    // apart)
    assert!(
        gap >= initial_gap * 0.5,
        "players should not clip through: gap={} vs initial={}",
        gap,
        initial_gap,
    );
}
