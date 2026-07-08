use bevy::prelude::*;

use super::*;
use crate::{
    core::identity::StableEntityId,
    demos::multiplayer_boxes::rope::RopeJointEntity,
    network::connection::{ClientSpawned, LocalPlayerId},
};

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
    app.insert_resource(LocalPlayerId(2))
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .insert_resource(crate::network::AfterglowNetworkContext::new(
            crate::network::LightyearRole::Client,
            2,
        ));
    app
}

/// Active RopeLink spawns a derived DistanceJoint.
#[test]
fn sync_rope_joints_creates_joint() {
    let mut app = rope_test_app();
    app.add_systems(Update, super::super::rope::sync_rope_joints);
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

#[test]
fn sync_rope_joints_creates_client_predicted_joint() {
    let mut app = rope_test_app();
    app.add_systems(Update, super::super::rope::sync_rope_joints);
    app.add_observer(super::super::rope::on_rope_link_removed);

    app.world_mut().spawn(ClientSpawned);
    let player = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "2".to_string(),
            },
            Transform::from_xyz(0.0, 0.4, 0.0),
            avian3d::prelude::RigidBody::Dynamic,
            lightyear::prelude::Predicted,
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
        player_owner: "2".to_string(),
        target: test_box_id(),
    });
    app.update();

    let joint_entity = app
        .world_mut()
        .query_filtered::<Entity, With<RopeJoint>>()
        .single(app.world())
        .expect("client predicted rope should create one joint");
    let joint = app
        .world()
        .get::<avian3d::prelude::DistanceJoint>(joint_entity)
        .unwrap();
    assert_eq!(joint.body1, player);
    assert_eq!(joint.body2, box_entity);
}

#[test]
fn sync_rope_joints_keeps_remote_predicted_rope_visual_only() {
    let mut app = rope_test_app();
    app.add_systems(Update, super::super::rope::sync_rope_joints);
    app.add_observer(super::super::rope::on_rope_link_removed);

    app.world_mut().spawn(ClientSpawned);
    app.world_mut().spawn((
        PlayerBox {
            owner: "99".to_string(),
        },
        Transform::from_xyz(0.0, 0.4, 0.0),
        avian3d::prelude::RigidBody::Dynamic,
        lightyear::prelude::Predicted,
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
            player_owner: "99".to_string(),
            target: test_box_id(),
        })
        .id();

    app.update();

    let joints: Vec<Entity> = app
        .world_mut()
        .query_filtered::<Entity, With<RopeJoint>>()
        .iter(app.world())
        .collect();
    assert!(
        joints.is_empty(),
        "remote predicted ropes should be visual-only on non-owning clients"
    );
    assert!(
        app.world().get::<RopeJointEntity>(link).is_none(),
        "remote rope link should not track a local physics joint"
    );
}

#[test]
fn sync_rope_joints_removes_orphan_joint_without_rope_marker() {
    let mut app = rope_test_app();
    app.add_systems(Update, super::super::rope::sync_rope_joints);

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
    let orphan_joint = app
        .world_mut()
        .spawn((
            RopeJoint,
            avian3d::prelude::DistanceJoint::new(player, box_entity),
        ))
        .id();

    app.update();

    assert!(
        app.world().get::<RopeJoint>(orphan_joint).is_none(),
        "orphan rope joints must be despawned so hidden/missing RopeLinks cannot keep boxes physically attached"
    );
}

#[test]
fn sync_rope_joints_recreates_joint_when_marker_points_to_despawned_entity() {
    let mut app = rope_test_app();
    app.add_systems(Update, super::super::rope::sync_rope_joints);
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

    let stale_joint = app.world().get::<RopeJointEntity>(link).unwrap().0;
    app.world_mut().entity_mut(stale_joint).despawn();
    app.update();

    let joints: Vec<Entity> = app
        .world_mut()
        .query_filtered::<Entity, With<RopeJoint>>()
        .iter(app.world())
        .collect();
    assert_eq!(joints.len(), 1, "stale joint should be recreated");
    assert_ne!(
        joints[0], stale_joint,
        "replacement joint should be a new entity"
    );
    assert_eq!(
        app.world().get::<RopeJointEntity>(link).unwrap().0,
        joints[0],
        "RopeJointEntity should point at the recreated joint"
    );
}

#[test]
fn sync_rope_joints_removes_stale_joint_when_player_despawns() {
    let mut app = rope_test_app();
    app.add_systems(Update, super::super::rope::sync_rope_joints);
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

    assert!(
        app.world().get::<RopeJointEntity>(link).is_some(),
        "active rope should track its derived joint"
    );
    app.world_mut().entity_mut(player).despawn();
    app.update();

    let count = app
        .world_mut()
        .query_filtered::<Entity, With<RopeJoint>>()
        .iter(app.world())
        .count();
    assert_eq!(count, 0, "stale joint should be despawned");
    assert!(
        app.world().get::<RopeJointEntity>(link).is_none(),
        "stale RopeJointEntity marker should be removed so the rope can recover"
    );
}

/// Removing RopeLink despawns the derived joint.
#[test]
fn sync_rope_joints_removes_joint_when_unroped() {
    let mut app = rope_test_app();
    app.add_systems(Update, super::super::rope::sync_rope_joints);
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
