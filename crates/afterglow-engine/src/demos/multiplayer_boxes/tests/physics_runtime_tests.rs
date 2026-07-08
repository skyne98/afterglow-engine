use std::time::Duration;

use avian3d::prelude::*;
use avian3d::dynamics::solver::joint_graph::JointGraph;
use bevy::{prelude::*, time::TimeUpdateStrategy};
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::Predicted;

use super::*;
use crate::input::AfterglowAction;

#[test]
fn client_prediction_physics_runs_without_contact_graph_panic() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::transform::TransformPlugin));
    app.insert_resource(crate::network::lightyear::AfterglowLightyearConfig {
        role: crate::network::lightyear::LightyearRole::Client,
        tick_rate: 60,
        ..Default::default()
    });
    app.add_plugins((
        crate::core::AfterglowCorePlugin,
        crate::network::AfterglowLightyearPlugin,
        crate::physics::AfterglowPhysicsPlugin,
    ));
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<Assets<StandardMaterial>>();
    app.add_systems(Startup, spawn_client_arena_visuals);
    app.add_systems(
        PreUpdate,
        (
            attach_predicted_player_physics,
            attach_predicted_kinematic_physics,
        ),
    );
    app.finish();
    app.cleanup();

    app.world_mut().spawn((
        PlayerBox {
            owner: "2".to_string(),
        },
        Transform::from_xyz(5.0, PLAYER_SIZE, 0.0),
        Predicted,
    ));
    for (i, pos) in [
        Vec3::new(-4.0, KINEMATIC_BOX_SIZE, -4.0),
        Vec3::new(4.0, KINEMATIC_BOX_SIZE, -4.0),
        Vec3::new(-4.0, KINEMATIC_BOX_SIZE, 4.0),
        Vec3::new(4.0, KINEMATIC_BOX_SIZE, 4.0),
        Vec3::new(-2.0, KINEMATIC_BOX_SIZE, 0.0),
        Vec3::new(2.0, KINEMATIC_BOX_SIZE, 0.0),
        Vec3::new(0.0, KINEMATIC_BOX_SIZE, -2.0),
        Vec3::new(0.0, KINEMATIC_BOX_SIZE, 2.0),
    ]
    .into_iter()
    .enumerate()
    {
        app.world_mut().spawn((
            KinematicBox { initial_pos: pos },
            StableEntityId::new(99_000 + i as u128),
            Transform::from_translation(pos),
            Predicted,
        ));
    }

    for _ in 0..120 {
        app.update();
    }
}

#[test]
fn live_avian_rope_joint_pulls_authoritative_block() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::transform::TransformPlugin));
    app.add_plugins(
        PhysicsPlugins::default()
            .build()
            .disable::<PhysicsTransformPlugin>()
            .disable::<PhysicsInterpolationPlugin>(),
    );
    app.add_plugins(afterglow_lightyear_avian3d::prelude::AfterglowAvianPlugin::default());
    app.add_systems(FixedUpdate, super::super::rope::sync_rope_joints);
    app.add_observer(super::super::rope::on_rope_link_removed);
    app.finish();
    app.cleanup();
    *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
        TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 60.0));
    app.world_mut().insert_resource(Gravity(Vec3::ZERO));

    let player = app
        .world_mut()
        .spawn((
            PlayerBox { owner: "1".into() },
            RigidBody::Dynamic,
            Collider::cuboid(PLAYER_SIZE * 2.0, PLAYER_SIZE * 2.0, PLAYER_SIZE * 2.0),
            Position::from(Vec3::new(0.0, PLAYER_SIZE, 0.0)),
            Rotation::default(),
            LinearVelocity(Vec3::NEG_X * PLAYER_SPEED),
            LockedAxes::ROTATION_LOCKED,
            Transform::from_xyz(0.0, PLAYER_SIZE, 0.0),
            ActionState::<AfterglowAction>::default(),
        ))
        .id();
    let block_id = StableEntityId::new(77_001);
    let block = app
        .world_mut()
        .spawn((
            KinematicBox {
                initial_pos: Vec3::new(2.0, KINEMATIC_BOX_SIZE, 0.0),
            },
            block_id,
            RigidBody::Dynamic,
            Collider::cuboid(
                KINEMATIC_BOX_SIZE * 2.0,
                KINEMATIC_BOX_SIZE * 2.0,
                KINEMATIC_BOX_SIZE * 2.0,
            ),
            Position::from(Vec3::new(2.0, KINEMATIC_BOX_SIZE, 0.0)),
            Rotation::default(),
            LinearVelocity::ZERO,
            LockedAxes::ROTATION_LOCKED,
            Transform::from_xyz(2.0, KINEMATIC_BOX_SIZE, 0.0),
        ))
        .id();
    app.world_mut().spawn(RopeLink {
        rope_id: StableEntityId::new(88_001),
        player_owner: "1".into(),
        target: block_id,
    });

    for _ in 0..180 {
        app.update();
    }

    let player_x = app.world().get::<Transform>(player).unwrap().translation.x;
    let block_x = app.world().get::<Transform>(block).unwrap().translation.x;
    assert!(player_x < -0.75, "player should move left, x={player_x}");
    assert!(
        block_x < 1.95,
        "live Avian rope joint should pull authoritative block left; block_x={block_x}, player_x={player_x}"
    );
}

/// Regression: after a RopeLink is despawned (authoritative unrope), the derived
/// Avian `DistanceJoint` must actually be removed from the physics `JointGraph`
/// across real physics steps. If the joint survives the despawn (ghost rope),
/// the block keeps following the player even though the rope is gone.
#[test]
fn despawned_rope_link_releases_block_across_physics_steps() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::transform::TransformPlugin));
    app.add_plugins(
        PhysicsPlugins::default()
            .build()
            .disable::<PhysicsTransformPlugin>()
            .disable::<PhysicsInterpolationPlugin>(),
    );
    app.add_plugins(afterglow_lightyear_avian3d::prelude::AfterglowAvianPlugin::default());
    app.add_systems(FixedUpdate, super::super::rope::sync_rope_joints);
    app.add_observer(super::super::rope::on_rope_link_removed);
    app.finish();
    app.cleanup();
    *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
        TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 60.0));
    app.world_mut().insert_resource(Gravity(Vec3::ZERO));

    let player = app
        .world_mut()
        .spawn((
            PlayerBox { owner: "1".into() },
            RigidBody::Dynamic,
            Collider::cuboid(PLAYER_SIZE * 2.0, PLAYER_SIZE * 2.0, PLAYER_SIZE * 2.0),
            Position::from(Vec3::new(0.0, PLAYER_SIZE, 0.0)),
            Rotation::default(),
            LinearVelocity(Vec3::NEG_X * PLAYER_SPEED),
            LockedAxes::ROTATION_LOCKED,
            Transform::from_xyz(0.0, PLAYER_SIZE, 0.0),
            ActionState::<AfterglowAction>::default(),
        ))
        .id();
    let block_id = StableEntityId::new(77_002);
    let block = app
        .world_mut()
        .spawn((
            KinematicBox {
                initial_pos: Vec3::new(2.0, KINEMATIC_BOX_SIZE, 0.0),
            },
            block_id,
            RigidBody::Dynamic,
            Collider::cuboid(
                KINEMATIC_BOX_SIZE * 2.0,
                KINEMATIC_BOX_SIZE * 2.0,
                KINEMATIC_BOX_SIZE * 2.0,
            ),
            Position::from(Vec3::new(2.0, KINEMATIC_BOX_SIZE, 0.0)),
            Rotation::default(),
            LinearVelocity::ZERO,
            LockedAxes::ROTATION_LOCKED,
            Transform::from_xyz(2.0, KINEMATIC_BOX_SIZE, 0.0),
        ))
        .id();
    let link = app
        .world_mut()
        .spawn(RopeLink {
            rope_id: StableEntityId::new(88_002),
            player_owner: "1".into(),
            target: block_id,
        })
        .id();

    // Establish the live joint is pulling the block.
    for _ in 0..120 {
        app.update();
    }
    let pre_release_block_x = app.world().get::<Transform>(block).unwrap().translation.x;
    let pre_release_player_x = app.world().get::<Transform>(player).unwrap().translation.x;
    assert!(
        pre_release_block_x < 1.95,
        "setup: joint should be pulling block left; block_x={pre_release_block_x}"
    );
    assert!(
        (pre_release_block_x - pre_release_player_x).abs() <= ROPE_MAX_DISTANCE + 0.2,
        "setup: block within rope length of player; block_x={pre_release_block_x}, player_x={pre_release_player_x}"
    );

    // Authoritative unrope: despawn the RopeLink. The on_rope_link_removed
    // observer and the sync_rope_joints orphan sweep must drop the joint.
    app.world_mut().entity_mut(link).despawn();

    // Decisive ghost-rope check: a surviving constraint would re-pull the
    // bodies back toward rope length regardless of where we place them. So
    // after despawn, separate the bodies far beyond rope length and zero both
    // velocities. If the joint is truly gone, separation is preserved (the
    // block stays put). If a hidden constraint survives, the block is
    // accelerated back toward the player and separation shrinks.
    app.update(); // let the despawn + on_remove observer apply
    {
        let rope_joint_count = app
            .world_mut()
            .query_filtered::<Entity, With<RopeJoint>>()
            .iter(app.world())
            .count();
        let distance_joint_count = app
            .world_mut()
            .query_filtered::<Entity, With<DistanceJoint>>()
            .iter(app.world())
            .count();
        let graph_joint_count = app
            .world()
            .resource::<JointGraph>()
            .joints_of(player)
            .count();
        assert_eq!(rope_joint_count, 0, "RopeJoint entity must be despawned");
        assert_eq!(distance_joint_count, 0, "DistanceJoint entity must be despawned");
        assert_eq!(graph_joint_count, 0, "JointGraph must drop the constraint");
    }

    // Place the block far to the right of the player, both at rest, well beyond
    // rope length and out of contact.
    app.world_mut()
        .entity_mut(player)
        .get_mut::<LinearVelocity>()
        .unwrap()
        .0 = Vec3::ZERO;
    app.world_mut()
        .entity_mut(block)
        .get_mut::<LinearVelocity>()
        .unwrap()
        .0 = Vec3::ZERO;
    app.world_mut()
        .entity_mut(player)
        .get_mut::<Position>()
        .unwrap()
        .0 = Vec3::new(0.0, PLAYER_SIZE, 0.0);
    app.world_mut()
        .entity_mut(block)
        .get_mut::<Position>()
        .unwrap()
        .0 = Vec3::new(20.0, KINEMATIC_BOX_SIZE, 0.0);
    app.world_mut()
        .entity_mut(player)
        .get_mut::<Transform>()
        .unwrap()
        .translation = Vec3::new(0.0, PLAYER_SIZE, 0.0);
    app.world_mut()
        .entity_mut(block)
        .get_mut::<Transform>()
        .unwrap()
        .translation = Vec3::new(20.0, KINEMATIC_BOX_SIZE, 0.0);

    // Run enough steps for a surviving constraint to manifest. With no joint,
    // the block at rest at x=20 stays at x=20 (gravity is zero, no input).
    for _ in 0..240 {
        app.update();
    }

    let player_x = app.world().get::<Transform>(player).unwrap().translation.x;
    let block_x = app.world().get::<Transform>(block).unwrap().translation.x;
    let separation = (player_x - block_x).abs();
    let block_vel_x = app.world().get::<LinearVelocity>(block).unwrap().x;
    assert!(
        separation > ROPE_MAX_DISTANCE + 5.0,
        "ghost rope regression: a surviving constraint would pull the block \
         back toward the player; separation={separation} should stay large; \
         player_x={player_x}, block_x={block_x}",
    );
    assert!(
        block_vel_x.abs() < 0.1,
        "ghost rope regression: released block at rest must not be accelerated; \
         block_vel_x={block_vel_x}"
    );
}
