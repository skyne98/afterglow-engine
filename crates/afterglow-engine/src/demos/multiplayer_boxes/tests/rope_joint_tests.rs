use bevy::prelude::*;

use super::*;

fn rope_test_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::input::InputPlugin,
        leafwing_input_manager::plugin::InputManagerPlugin::<crate::input::AfterglowAction>::default(),
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

// Sync rope joints tests
// ---------------------------------------------------------------------------

/// Adding RopedTo spawns a DistanceJoint (via observer).
#[test]
fn sync_rope_joints_creates_joint() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();

    // Register observers BEFORE spawning entities with RopedTo
    app.add_observer(super::super::rope::on_roped_to_added);
    app.add_observer(super::super::rope::on_roped_to_removed);

    let player = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "alice".to_string(),
            },
            Transform::from_xyz(0.0, 0.4, 0.0),
            avian3d::prelude::RigidBody::Dynamic,
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
            RopedTo {
                player_owner: "alice".to_string(),
            },
        ))
        .id();

    app.add_observer(super::super::rope::on_roped_to_added);
    app.add_observer(super::super::rope::on_roped_to_removed);
    app.update();

    let joints: Vec<Entity> = app
        .world_mut()
        .query_filtered::<Entity, With<RopeJoint>>()
        .iter(app.world())
        .collect();
    assert_eq!(joints.len(), 1, "should be exactly one rope joint");

    let joint = app
        .world()
        .get::<avian3d::prelude::DistanceJoint>(joints[0])
        .unwrap();
    assert_eq!(joint.body1, player, "joint body1 should be the player");
    assert_eq!(joint.body2, box_entity, "joint body2 should be the box");
}

/// Removing RopedTo despawns the joint (via observer).
#[test]
fn sync_rope_joints_removes_joint_when_unroped() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();

    // Register observers BEFORE spawning entities with RopedTo
    app.add_observer(super::super::rope::on_roped_to_added);
    app.add_observer(super::super::rope::on_roped_to_removed);

    app.world_mut().spawn((
        PlayerBox {
            owner: "alice".to_string(),
        },
        Transform::from_xyz(0.0, 0.4, 0.0),
        avian3d::prelude::RigidBody::Dynamic,
    ));

    let box_entity = app
        .world_mut()
        .spawn((
            KinematicBox {
                id: 0,
                initial_pos: Vec3::new(1.0, 0.5, 0.0),
            },
            Transform::from_xyz(1.0, 0.5, 0.0),
            avian3d::prelude::RigidBody::Dynamic,
            RopedTo {
                player_owner: "alice".to_string(),
            },
        ))
        .id();

    app.add_observer(super::super::rope::on_roped_to_added);
    app.add_observer(super::super::rope::on_roped_to_removed);
    app.update();

    let count = app
        .world_mut()
        .query_filtered::<Entity, With<RopeJoint>>()
        .iter(app.world())
        .count();
    assert_eq!(count, 1);

    app.world_mut().entity_mut(box_entity).remove::<RopedTo>();
    app.update();

    let count = app
        .world_mut()
        .query_filtered::<Entity, With<RopeJoint>>()
        .iter(app.world())
        .count();
    assert_eq!(count, 0, "joint should be despawned after unroping");
}
