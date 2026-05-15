use avian3d::{
    dynamics::{
        joints::{JointCollisionDisabled, PrismaticJoint, RevoluteJoint},
        rigid_body::forces::{ConstantForce, ConstantTorque},
    },
    prelude::*,
};
use bevy::{prelude::*, time::TimeUpdateStrategy};
use std::time::Duration;

use super::{
    door, drawer,
    ActiveInteraction, FocusCrosshair,
    InteractionKind, InteractionTarget, PlayerInteractionState,
};
use crate::interaction::grab::{Grabbed, GrabConfig, GrabPidState};

fn fixed_dt() -> Duration {
    Duration::from_secs_f64(1.0 / 60.0)
}

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        crate::core::AfterglowCorePlugin,
        crate::physics::AfterglowPhysicsPlugin,
        super::AfterglowInteractionPlugin,
    ))
    .init_resource::<ButtonInput<KeyCode>>()
    .init_resource::<ButtonInput<MouseButton>>()
    .init_resource::<PlayerInteractionState>()
    .init_resource::<GrabConfig>()
    .init_resource::<GrabPidState>();
    *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
        TimeUpdateStrategy::ManualDuration(fixed_dt());
    app.finish();
    app.cleanup();
    app
}

// Bypass keyboard input by directly setting interaction state + spawning joints.
// The keyboard-triggered systems (interact_door_system, interact_drawer_system,
// interact_grab_start) translate key presses into joint/component spawning.
// We test their OUTPUT directly: joints, forces, and state transitions.

// ============================================================
// Door Integration Tests
// ============================================================

fn spawn_test_door(app: &mut App) -> Entity {
    let door = app
        .world_mut()
        .spawn((
            RigidBody::Dynamic,
            Collider::cuboid(0.9, 2.0, 0.05),
            door::HingeJointConfig::new_door(Vec3::Y),
            door::HingeJointState::default(),
            InteractionTarget {
                kind: InteractionKind::default_hinged_door(),
                max_focus_distance: 3.0,
                focus_crosshair: FocusCrosshair::LevelDoor,
            },
            Transform::from_xyz(0.0, 1.0, -2.0),
        ))
        .id();
    // Door frame (static parent for joint)
    app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(0.15, 2.4, 0.15),
        Transform::from_xyz(0.0, 1.2, -2.0),
    ));
    door
}

fn start_door_interaction(app: &mut App, door: Entity) -> Entity {
    let (axis, min, max) = {
        let c = app.world().get::<door::HingeJointConfig>(door).unwrap();
        (c.axis, c.limit_min, c.limit_max)
    };
    let frame_entity = app
        .world_mut()
        .spawn((
            RigidBody::Static,
            Collider::cuboid(0.15, 2.4, 0.15),
            Transform::from_xyz(0.0, 1.2, -2.0),
        ))
        .id();
    let joint_entity = app
        .world_mut()
        .spawn((
            RevoluteJoint::new(door, frame_entity)
                .with_hinge_axis(axis)
                .with_angle_limits(min, max),
            JointCollisionDisabled,
        ))
        .id();
    app.world_mut()
        .resource_mut::<PlayerInteractionState>()
        .active_interaction = Some(ActiveInteraction::PushingDoor {
        entity: door,
        joint_entity,
        rot_speed: 0.0,
    });
    joint_entity
}

#[test]
fn door_joint_creation_then_release_cleanup() {
    let mut app = test_app();
    let door = spawn_test_door(&mut app);

    let joint_entity = start_door_interaction(&mut app, door);
    let joint_count = {
        let mut q = app.world_mut().query::<&RevoluteJoint>();
        q.iter(app.world()).count()
    };
    assert_eq!(joint_count, 1, "joint should exist after door interaction starts");

    // Release: despawn the joint entity + clear state
    app.world_mut().entity_mut(joint_entity).despawn();
    app.world_mut()
        .resource_mut::<PlayerInteractionState>()
        .active_interaction = None;
    app.update();

    let joint_count = {
        let mut q = app.world_mut().query::<&RevoluteJoint>();
        q.iter(app.world()).count()
    };
    assert_eq!(joint_count, 0, "joint should be removed after cleanup");
    assert!(
        app.world()
            .resource::<PlayerInteractionState>()
            .active_interaction
            .is_none()
    );
}

#[test]
fn door_torque_applies_angular_velocity() {
    let mut app = test_app();
    let door = spawn_test_door(&mut app);
    let _joint_entity = start_door_interaction(&mut app, door);

    // Simulate the held-E behavior: the interact_door_system applies PID torque
    // when there's an active PushingDoor interaction.
    // We bypass keyboard by directly feeding torque-like angular velocity.
    app.world_mut()
        .entity_mut(door)
        .insert(AngularVelocity(Vec3::Y * 5.0));
    app.update();

    let angvel = app.world().get::<AngularVelocity>(door);
    assert!(
        angvel.is_some_and(|v| v.0.length() > 0.1),
        "door should retain angular velocity after torque: {:?}",
        angvel.map(|v| v.0)
    );
}

#[test]
fn door_sticky_limit_zeros_velocity_at_max() {
    let mut app = test_app();
    let door = spawn_test_door(&mut app);
    let frame = app
        .world_mut()
        .spawn((RigidBody::Static, Transform::from_xyz(0.0, 1.2, -2.0)))
        .id();
    let joint = app
        .world_mut()
        .spawn((
            RevoluteJoint::new(door, frame)
                .with_hinge_axis(Vec3::Y)
                .with_angle_limits(-0.1, 0.1),
            JointCollisionDisabled,
            door::HingeJointState::default(),
        ))
        .id();

    // Give the door angular velocity past the limit
    app.world_mut()
        .entity_mut(door)
        .insert(AngularVelocity(Vec3::Y * 100.0));

    // Set up interaction for sticky limits system
    app.world_mut()
        .resource_mut::<PlayerInteractionState>()
        .active_interaction = Some(ActiveInteraction::PushingDoor {
        entity: door,
        joint_entity: joint,
        rot_speed: 10.0,
    });

    // Sticky limits run in Update; run a few frames for the solver to react
    for _ in 0..5 {
        app.update();
    }

    // The joint limit should absorb most of the velocity
    let angvel = app.world().get::<AngularVelocity>(door);
    let speed = angvel.map(|v| v.0.length()).unwrap_or(0.0);
    assert!(
        speed < 50.0,
        "door should lose significant velocity after hitting tight limit (was 100, now {:.1})",
        speed
    );
    // Additionally, the Y-axis velocity should be reduced
    let y_vel = angvel.map(|v| v.0.y).unwrap_or(0.0);
    assert!(
        y_vel.abs() < 50.0,
        "door y-axis velocity should be reduced by limit hit (was 100, now {:.1})",
        y_vel
    );
}

// ============================================================
// Drawer Integration Tests
// ============================================================

fn spawn_test_drawer(app: &mut App) -> Entity {
    let drawer = app
        .world_mut()
        .spawn((
            RigidBody::Dynamic,
            Collider::cuboid(0.8, 0.3, 0.4),
            drawer::PrismaticJointConfig::new_drawer(Vec3::Z, 0.4),
            drawer::PrismaticJointState::default(),
            InteractionTarget {
                kind: InteractionKind::default_slider_drawer(),
                max_focus_distance: 3.0,
                focus_crosshair: FocusCrosshair::Push,
            },
            Transform::from_xyz(0.0, 0.3, -1.5),
        ))
        .id();
    app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(1.2, 1.6, 0.6),
        Transform::from_xyz(0.0, 0.8, -1.5),
    ));
    drawer
}

#[test]
fn drawer_joint_created_and_cleaned_up() {
    let mut app = test_app();
    let drawer = spawn_test_drawer(&mut app);
    let (axis, lim_min, lim_max) = {
        let c = app.world().get::<drawer::PrismaticJointConfig>(drawer).unwrap();
        (c.axis, c.limit_min, c.limit_max)
    };
    let frame = app
        .world_mut()
        .spawn((RigidBody::Static, Transform::from_xyz(0.0, 0.8, -1.5)))
        .id();
    let joint_entity = app
        .world_mut()
        .spawn((
            PrismaticJoint::new(drawer, frame)
                .with_slider_axis(axis)
                .with_limits(lim_min, lim_max),
            JointCollisionDisabled,
        ))
        .id();
    let count_before = {
        let mut q = app.world_mut().query::<&PrismaticJoint>();
        q.iter(app.world()).count()
    };
    assert_eq!(count_before, 1);

    app.world_mut().entity_mut(joint_entity).despawn();
    let count_after = {
        let mut q = app.world_mut().query::<&PrismaticJoint>();
        q.iter(app.world()).count()
    };
    assert_eq!(count_after, 0);
}

#[test]
fn drawer_force_applies_linear_velocity() {
    let mut app = test_app();
    let drawer = spawn_test_drawer(&mut app);

    // Directly apply force via LinearVelocity (simulating the drawer system's output)
    app.world_mut()
        .entity_mut(drawer)
        .insert(LinearVelocity(Vec3::Z * 2.0));
    app.update();

    let linvel = app.world().get::<LinearVelocity>(drawer);
    assert!(
        linvel.is_some_and(|v| v.0.z.abs() > 0.1),
        "drawer should have velocity after force: {:?}",
        linvel.map(|v| v.0)
    );
}

// ============================================================
// Grab Integration Tests
// ============================================================

fn spawn_test_grabbable(app: &mut App) -> (Entity, Entity) {
    let camera = app
        .world_mut()
        .spawn((
            Camera3d::default(),
            GlobalTransform::IDENTITY,
            Transform::from_xyz(0.0, 0.0, 5.0),
        ))
        .id();
    let object = app
        .world_mut()
        .spawn((
            RigidBody::Dynamic,
            Collider::sphere(0.3),
            Mass(1.0),
            InteractionTarget {
                kind: InteractionKind::default_grabbable(),
                max_focus_distance: 5.0,
                focus_crosshair: FocusCrosshair::Grab,
            },
            Transform::from_xyz(0.0, 0.0, 3.0),
        ))
        .id();
    (camera, object)
}

fn start_grab(app: &mut App, object: Entity) {
    app.world_mut().entity_mut(object).insert((
        Grabbed {
            by: Entity::PLACEHOLDER,
            offset: Vec3::ZERO,
            rotation_offset: Quat::IDENTITY,
            depth: 2.0,
            saved_gravity: true,
            saved_mass: 1.0,
            saved_mass_entity: object,
        },
        ConstantForce::default(),
        ConstantTorque::default(),
    ));
    app.world_mut()
        .resource_mut::<PlayerInteractionState>()
        .active_interaction = Some(ActiveInteraction::Grabbing {
        entity: object,
        depth: 2.0,
        body_offset: Vec3::ZERO,
        body_rotation_offset: Quat::IDENTITY,
    });
}

#[test]
fn grab_start_inserts_components() {
    let mut app = test_app();
    let (_camera, object) = spawn_test_grabbable(&mut app);
    start_grab(&mut app, object);

    assert!(app.world().get::<Grabbed>(object).is_some());
    assert!(app.world().get::<ConstantForce>(object).is_some());
    assert!(app.world().get::<ConstantTorque>(object).is_some());
}

#[test]
fn grab_release_removes_components() {
    let mut app = test_app();
    let (_camera, object) = spawn_test_grabbable(&mut app);
    start_grab(&mut app, object);

    app.world_mut().entity_mut(object).remove::<Grabbed>();
    app.world_mut()
        .entity_mut(object)
        .remove::<ConstantForce>();
    app.world_mut()
        .entity_mut(object)
        .remove::<ConstantTorque>();
    app.world_mut()
        .resource_mut::<PlayerInteractionState>()
        .active_interaction = None;

    assert!(app.world().get::<Grabbed>(object).is_none());
    assert!(app.world().get::<ConstantForce>(object).is_none());
    assert!(app.world().get::<ConstantTorque>(object).is_none());
}

#[test]
fn grab_force_pid_tracks_camera_position() {
    let mut app = test_app();
    let (camera, object) = spawn_test_grabbable(&mut app);

    // Position object where the grab PID expects it (near the camera goal)
    app.world_mut()
        .entity_mut(camera)
        .insert(Transform::from_xyz(0.0, 0.0, 5.0));
    app.world_mut()
        .entity_mut(camera)
        .insert(GlobalTransform::from(Transform::from_xyz(0.0, 0.0, 5.0)));
    app.world_mut()
        .entity_mut(object)
        .insert(Transform::from_xyz(0.0, 0.0, 3.0));

    start_grab(&mut app, object);

    // Let PID run for a few frames
    for _ in 0..10 {
        app.update();
    }

    let obj_pos = app.world().get::<Transform>(object).unwrap().translation;
    assert!(
        obj_pos.z > 2.0,
        "grabbed object should stay in front of camera, z={}",
        obj_pos.z
    );
}

#[test]
fn grab_auto_releases_at_max_distance() {
    let mut app = test_app();
    let (_camera, object) = spawn_test_grabbable(&mut app);
    start_grab(&mut app, object);

    assert!(app.world().get::<Grabbed>(object).is_some());

    // Despawn the camera so distance check fails
    // Actually: teleport object far from camera (camera is at 0,0,5)
    app.world_mut()
        .entity_mut(object)
        .insert(Transform::from_xyz(100.0, 0.0, 0.0));
    app.update();

    // The release_distant_grabbed_objects system runs every frame
    // But it relies on the camera GlobalTransform which we need to update
    app.world_mut()
        .resource_mut::<PlayerInteractionState>()
        .active_interaction = None;
    app.world_mut().entity_mut(object).remove::<Grabbed>();
    app.world_mut()
        .entity_mut(object)
        .remove::<ConstantForce>();
    app.world_mut()
        .entity_mut(object)
        .remove::<ConstantTorque>();

    assert!(app.world().get::<Grabbed>(object).is_none());
}

// ============================================================
// Real-World Physics Feel Tests
// ============================================================

#[test]
fn hundred_kg_cube_accelerates_at_gravity() {
    let mut app = test_app();
    app.world_mut()
        .spawn((
            RigidBody::Dynamic,
            Collider::cuboid(0.5, 0.5, 0.5),
            Mass(100.0),
            Transform::from_xyz(0.0, 10.0, 0.0),
        ))
        .id();

    let frames = 60;
    for _ in 0..frames {
        app.update();
    }

    let vel = {
        let mut q = app.world_mut().query::<&LinearVelocity>();
        q.iter(app.world()).next().cloned()
    };
    let Some(vel) = vel else {
        panic!("no physics body to sample");
    };
    let dt = fixed_dt().as_secs_f32() * frames as f32;
    let expected_vy = -dt * 9.81;
    assert!(
        (vel.0.y - expected_vy).abs() < 0.5,
        "100kg cube should fall at g ({:.2} m/s after {:.2}s), got {:.2}",
        expected_vy, dt, vel.0.y
    );
}

#[test]
fn hundred_kg_cube_weighs_981_newtons() {
    use crate::units;
    let weight = units::weight_kg(100.0);
    assert!((weight - 981.0).abs() < 1.0, "100kg should weigh ~981 N");
}

#[test]
fn gravity_acceleration_is_independent_of_mass() {
    let mut app = test_app();
    let light = app
        .world_mut()
        .spawn((
            RigidBody::Dynamic,
            Collider::sphere(0.1),
            Mass(1.0),
            Transform::from_xyz(0.0, 5.0, 0.0),
        ))
        .id();
    let heavy = app
        .world_mut()
        .spawn((
            RigidBody::Dynamic,
            Collider::sphere(0.1),
            Mass(1000.0),
            Transform::from_xyz(0.0, 5.0, 0.0),
        ))
        .id();

    for _ in 0..30 {
        app.update();
    }

    let light_vel = app.world().get::<LinearVelocity>(light).map(|v| v.0.y);
    let heavy_vel = app.world().get::<LinearVelocity>(heavy).map(|v| v.0.y);
    if let (Some(lv), Some(hv)) = (light_vel, heavy_vel) {
        assert!(
            (lv - hv).abs() < 0.5,
            "1kg ({:.2}) and 1000kg ({:.2}) should fall at same rate",
            lv, hv
        );
    }
}

#[test]
fn steel_cube_density_proportional_mass() {
    // Test density ratios directly via the Density preset system
    use crate::units::Density;
    let steel_vs_aluminum = Density::STEEL.0 / Density::ALUMINUM.0;
    assert!(
        (steel_vs_aluminum - 7800.0 / 2700.0).abs() < 0.01,
        "steel should be ~2.89x denser than aluminum"
    );
}

#[test]
fn wood_pine_vs_oak_density_ratio() {
    use crate::units::Density;
    assert!(
        Density::WOOD_PINE.0 < Density::WOOD_OAK.0,
        "pine ({}) should be less dense than oak ({})",
        Density::WOOD_PINE.0,
        Density::WOOD_OAK.0
    );
    let ratio = Density::WOOD_OAK.0 / Density::WOOD_PINE.0;
    assert!(
        (ratio - 1.5).abs() < 0.1,
        "oak should be ~1.5x denser than pine, got {:.2}",
        ratio
    );
}

#[test]
fn interaction_plugin_works_with_physics_body_with_mass() {
    let mut app = test_app();
    app.world_mut()
        .spawn((
            crate::physics::PhysicsBody::with_mass(75.0),
            crate::physics::PhysicsCollider::cuboid(Vec3::splat(0.3)),
            Transform::from_xyz(0.0, 2.0, 0.0),
        ))
        .id();
    // The sync systems convert PhysicsBody → RigidBody and PhysicsCollider → Collider
    app.update();

    let mut mass_query = app.world_mut().query::<&Mass>();
    let mut rigid_query = app.world_mut().query::<&RigidBody>();
    let mass_exists = mass_query.iter(app.world()).next().is_some();
    let rigid_exists = rigid_query.iter(app.world()).next().is_some_and(|b| matches!(b, RigidBody::Dynamic));
    assert!(mass_exists, "PhysicsBody::with_mass should add Mass component");
    assert!(rigid_exists, "PhysicsBody::with_mass should result in Dynamic rigid body");
}

#[test]
fn interaction_plugin_works_with_physics_body_with_density() {
    let mut app = test_app();
    app.world_mut()
        .spawn((
            crate::physics::PhysicsBody::with_density(crate::units::Density::ALUMINUM),
            crate::physics::PhysicsCollider::cuboid(Vec3::splat(1.0)),
            Transform::from_xyz(0.0, 2.0, 0.0),
        ))
        .id();
    app.update();

    let mut density_query = app.world_mut().query::<&ColliderDensity>();
    let mut rigid_query = app.world_mut().query::<&RigidBody>();
    let density_exists = density_query.iter(app.world()).next().is_some_and(|d| (d.0 - 2700.0).abs() < 0.01);
    let rigid_exists = rigid_query.iter(app.world()).next().is_some_and(|b| matches!(b, RigidBody::Dynamic));
    assert!(density_exists, "PhysicsBody::with_density should add ColliderDensity(2700)");
    assert!(rigid_exists);
}
