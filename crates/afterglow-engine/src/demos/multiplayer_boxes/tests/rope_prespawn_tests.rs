use std::time::Duration;

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::*;

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
    super::super::network::register_demo_protocol(&mut app);
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

/// Client-only local input does not pre-spawn a RopeLink until a RopeIntent
/// sender exists, because an unsent intent can never be confirmed.
#[test]
fn client_local_release_without_sender_does_not_prespawn_rope_link() {
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
    app.world_mut().spawn((
        KinematicBox {
            id: 0,
            initial_pos: Vec3::new(1.0, 0.5, 0.0),
        },
        Transform::from_xyz(1.0, 0.5, 0.0),
        avian3d::prelude::RigidBody::Dynamic,
        LinearVelocity::ZERO,
    ));

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
        rope_link_for_box(&mut app, 0).is_none(),
        "client must not pre-spawn an unconfirmable RopeLink without a sender"
    );
}

/// Client-only local input sends a RopeIntent and pre-spawns a RopeLink.
#[test]
fn client_local_release_sends_intent_and_prespawns_rope_link() {
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
    let client_link = app
        .world_mut()
        .spawn(MessageSender::<RopeIntent>::default())
        .id();
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

/// Client detach sends an intent and immediately clears the predicted RopeLink
/// so the client is not blocked waiting for server correction.
#[test]
fn client_local_detach_sends_intent_and_despawns_predicted_rope_link() {
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
    let client_link = app
        .world_mut()
        .spawn(MessageSender::<RopeIntent>::default())
        .id();
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
    app.world_mut().spawn((
        KinematicBox {
            id: 0,
            initial_pos: Vec3::new(1.0, 0.5, 0.0),
        },
        Transform::from_xyz(1.0, 0.5, 0.0),
        avian3d::prelude::RigidBody::Dynamic,
        LinearVelocity::ZERO,
    ));

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
    assert!(rope_link_for_box(&mut app, 0).is_some());

    for _ in 0..12 {
        app.update();
    }
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
        rope_link_for_box(&mut app, 0).is_none(),
        "client detach should immediately clear the predicted RopeLink"
    );
}

/// The authoritative side validates the client-selected box id and confirms
/// the matching PreSpawned RopeLink instead of recomputing a target.
#[test]
fn server_rope_intent_confirms_exact_client_selected_box() {
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

    app.world_mut().spawn((
        PlayerBox {
            owner: "2".to_string(),
        },
        Transform::from_xyz(0.0, 0.4, 0.0),
        avian3d::prelude::RigidBody::Dynamic,
        LinearVelocity::ZERO,
        ActionState::<AfterglowAction>::default(),
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

    app.add_systems(Update, apply_test_attach_intent);
    app.update();

    let roped = rope_link_for_box(&mut app, 0);
    assert_eq!(roped.as_ref().map(|r| r.player_owner.as_str()), Some("2"));
    assert_eq!(
        prespawned_rope_link_count(&mut app),
        1,
        "authoritative RopeLink should carry matching PreSpawned metadata"
    );
}

fn apply_test_attach_intent(
    commands: Commands,
    players: Query<(&PlayerBox, &Transform)>,
    boxes: Query<(&KinematicBox, &Transform)>,
    links: Query<(Entity, &RopeLink)>,
) {
    super::super::rope::apply_authoritative_rope_intent(
        commands,
        "2".to_string(),
        RopeIntent {
            op: RopeIntentOp::Attach,
            box_id: Some(0),
        },
        &players,
        &boxes,
        &links,
    );
}
