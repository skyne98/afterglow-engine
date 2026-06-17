use std::time::Duration;

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::{PreSpawned, Predicted};

use super::*;
use crate::input::{AfterglowAction, default_gameplay_input_map};

fn rope_test_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::input::InputPlugin,
        lightyear::prelude::server::ServerPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / 60.0),
        },
        leafwing_input_manager::plugin::InputManagerPlugin::<AfterglowAction>::default(),
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

fn rope_link_for_box(app: &mut App, box_id: u32) -> Option<RopeLink> {
    app.world_mut()
        .query::<&RopeLink>()
        .iter(app.world())
        .find(|link| link.box_id == box_id)
        .cloned()
}

fn prespawned_rope_link_count(app: &mut App) -> usize {
    app.world_mut()
        .query::<(&RopeLink, &PreSpawned)>()
        .iter(app.world())
        .count()
}

/// Client-only local input does not mutate replicated rope state directly.
/// The server consumes the client's ActionState and writes authoritative
/// RopeLink, avoiding component prediction/correction attach-drop races.
#[test]
fn client_local_release_does_not_write_roped_to() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "bob".to_string();
    app.world_mut()
        .insert_resource(crate::network::AfterglowNetworkContext::from_status(
            crate::network::AfterglowConnectionStatus {
                role: crate::network::LightyearRole::Client,
                local_member_id: crate::network::session::SessionMemberId::new(2),
                ..Default::default()
            },
        ));
    let client_link = app.world_mut().spawn_empty().id();
    app.world_mut()
        .insert_resource(crate::network::SessionLightyearLinks {
            client_link: Some(client_link),
            ..Default::default()
        });

    app.world_mut().spawn((
        PlayerBox {
            owner: "2".to_string(),
        },
        Transform::from_xyz(0.0, 0.4, 0.0),
        avian3d::prelude::RigidBody::Dynamic,
        LinearVelocity::ZERO,
        default_gameplay_input_map(),
        ActionState::<AfterglowAction>::default(),
        Predicted,
    ));

    let _box_entity = app
        .world_mut()
        .spawn((
            KinematicBox {
                id: 0,
                initial_pos: Vec3::new(1.0, 0.5, 0.0),
            },
            Transform::from_xyz(1.0, 0.5, 0.0),
            avian3d::prelude::RigidBody::Dynamic,
            LinearVelocity::ZERO,
        ))
        .id();

    app.add_systems(
        PreUpdate,
        super::super::rope::toggle_rope
            .after(leafwing_input_manager::plugin::InputManagerSystem::Update),
    );
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyF);
    app.update();
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyF);
    app.update();

    assert!(
        rope_link_for_box(&mut app, 0).is_some(),
        "client prediction should pre-spawn a local RopeLink entity"
    );
    assert_eq!(
        prespawned_rope_link_count(&mut app),
        1,
        "predicted RopeLink should use Lightyear PreSpawned"
    );
}
/// The authoritative host consumes a remote client's replicated ActionState
/// release edge and creates the server-owned RopeLink entity. This confirms
/// the client's pre-spawned rope prediction.
#[test]
fn server_remote_action_state_release_ropes_nearest_box() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();
    app.world_mut()
        .insert_resource(crate::network::AfterglowNetworkContext::from_status(
            crate::network::AfterglowConnectionStatus {
                role: crate::network::LightyearRole::Host,
                local_member_id: crate::network::session::SessionMemberId::new(1),
                ..Default::default()
            },
        ));

    let remote = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "2".to_string(),
            },
            Transform::from_xyz(0.0, 0.4, 0.0),
            avian3d::prelude::RigidBody::Dynamic,
            LinearVelocity::ZERO,
            ActionState::<AfterglowAction>::default(),
        ))
        .id();

    let _box_entity = app
        .world_mut()
        .spawn((
            KinematicBox {
                id: 0,
                initial_pos: Vec3::new(1.0, 0.5, 0.0),
            },
            Transform::from_xyz(1.0, 0.5, 0.0),
            avian3d::prelude::RigidBody::Dynamic,
            LinearVelocity::ZERO,
        ))
        .id();

    app.add_systems(
        Update,
        super::super::rope::server_toggle_remote_ropes_from_inputs,
    );

    app.world_mut()
        .get_mut::<ActionState<AfterglowAction>>(remote)
        .unwrap()
        .press(&AfterglowAction::RopeToggle);
    app.update();
    app.update();
    assert!(rope_link_for_box(&mut app, 0).is_none());

    app.world_mut()
        .get_mut::<ActionState<AfterglowAction>>(remote)
        .unwrap()
        .release(&AfterglowAction::RopeToggle);
    app.update();

    let roped = rope_link_for_box(&mut app, 0);
    assert_eq!(roped.as_ref().map(|r| r.player_owner.as_str()), Some("2"));
    assert_eq!(
        prespawned_rope_link_count(&mut app),
        1,
        "authoritative RopeLink should carry matching PreSpawned metadata"
    );

    // The server may observe stale/replayed states immediately after one
    // release. A false->true->false replay inside the cooldown must not toggle
    // the rope back off.
    app.world_mut()
        .get_mut::<ActionState<AfterglowAction>>(remote)
        .unwrap()
        .press(&AfterglowAction::RopeToggle);
    app.update();
    app.world_mut()
        .get_mut::<ActionState<AfterglowAction>>(remote)
        .unwrap()
        .release(&AfterglowAction::RopeToggle);
    app.update();
    assert_eq!(
        rope_link_for_box(&mut app, 0)
            .as_ref()
            .map(|r| r.player_owner.as_str()),
        Some("2")
    );
}
