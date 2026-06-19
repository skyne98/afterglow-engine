use bevy::prelude::*;

use super::*;
use crate::core::identity::StableEntityId;

fn test_box_id() -> StableEntityId {
    StableEntityId::new(10_000)
}

fn test_rope_id() -> StableEntityId {
    StableEntityId::new(20_000)
}

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

/// Adding RopeLink spawns a DistanceJoint (via observer).
#[test]
fn sync_rope_joints_creates_joint() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();
    app.add_observer(super::super::rope::on_rope_link_added);
    app.add_observer(super::super::rope::on_rope_link_removed);

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
                initial_pos: Vec3::new(1.0, 0.5, 0.0),
            },
            test_box_id(),
            Transform::from_xyz(1.0, 0.5, 0.0),
            avian3d::prelude::RigidBody::Dynamic,
        ))
        .id();

    app.world_mut().spawn(RopeLink {
        rope_id: test_rope_id(),
        player_owner: "alice".to_string(),
        target: test_box_id(),
    });
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

/// Removing RopeLink despawns the joint (via observer).
#[test]
fn sync_rope_joints_removes_joint_when_unroped() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();
    app.add_observer(super::super::rope::on_rope_link_added);
    app.add_observer(super::super::rope::on_rope_link_removed);

    app.world_mut().spawn((
        PlayerBox {
            owner: "alice".to_string(),
        },
        Transform::from_xyz(0.0, 0.4, 0.0),
        avian3d::prelude::RigidBody::Dynamic,
    ));

    app.world_mut().spawn((
        KinematicBox {
            initial_pos: Vec3::new(1.0, 0.5, 0.0),
        },
        test_box_id(),
        Transform::from_xyz(1.0, 0.5, 0.0),
        avian3d::prelude::RigidBody::Dynamic,
    ));

    let link = app
        .world_mut()
        .spawn(RopeLink {
            rope_id: test_rope_id(),
            player_owner: "alice".to_string(),
            target: test_box_id(),
        })
        .id();
    app.update();

    let count = app
        .world_mut()
        .query_filtered::<Entity, With<RopeJoint>>()
        .iter(app.world())
        .count();
    assert_eq!(count, 1);

    app.world_mut().entity_mut(link).despawn();
    app.update();

    let count = app
        .world_mut()
        .query_filtered::<Entity, With<RopeJoint>>()
        .iter(app.world())
        .count();
    assert_eq!(count, 0, "joint should be despawned after unroping");
}
