use std::time::Duration;

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::{PreSpawned, Predicted, PredictionDisable};

use super::*;
use crate::{
    core::identity::StableEntityId,
    input::{AfterglowAction, default_gameplay_input_map},
    network::connection::{ClientSpawned, LocalPlayerId},
};

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
    app.insert_resource(LocalPlayerId(2))
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .insert_resource(crate::network::AfterglowNetworkContext::new(
            crate::network::LightyearRole::Client,
            2,
        ));
    app
}

fn run_rope_frame(app: &mut App) {
    app.world_mut().run_schedule(FixedUpdate);
    app.world_mut().run_schedule(Update);
    app.world_mut().run_schedule(PostUpdate);
}

fn release_rope_toggle(app: &mut App) {
    let mut world = app.world_mut();
    let mut query = world.query::<&mut ActionState<AfterglowAction>>();
    for mut action in query.iter_mut(&mut world) {
        action.press(&AfterglowAction::RopeToggle);
        action.release(&AfterglowAction::RopeToggle);
    }
}

fn test_box_id(index: u32) -> StableEntityId {
    StableEntityId::new(10_000 + u128::from(index))
}

fn active_visible_rope_count(app: &mut App, owner: &str) -> usize {
    app.world_mut()
        .query::<(
            &RopeLink,
            Option<&super::super::rope::RopeJointDetachPending>,
            Option<&PredictionDisable>,
        )>()
        .iter(app.world())
        .filter(|(link, detach_pending, disabled)| {
            link.player_owner == owner && detach_pending.is_none() && disabled.is_none()
        })
        .count()
}

fn spawn_predicted_local_player_and_box(app: &mut App) -> (Entity, Entity) {
    let player = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "2".to_string(),
            },
            Transform::from_xyz(0.0, 0.4, 0.0),
            avian3d::prelude::RigidBody::Dynamic,
            LinearVelocity::ZERO,
            default_gameplay_input_map(),
            ActionState::<AfterglowAction>::default(),
            Predicted,
        ))
        .id();

    let box_entity = app
        .world_mut()
        .spawn((
            KinematicBox {
                initial_pos: Vec3::new(1.0, 0.5, 0.0),
            },
            test_box_id(0),
            Transform::from_xyz(1.0, 0.5, 0.0),
            avian3d::prelude::RigidBody::Dynamic,
            LinearVelocity::ZERO,
        ))
        .id();

    (player, box_entity)
}

fn spawn_client_rope(app: &mut App, client_link: Entity) -> Entity {
    let target = test_box_id(0);
    let rope_id = super::super::rope::rope_id_for_input("2", target);
    app.world_mut()
        .spawn((
            RopeLink {
                rope_id,
                player_owner: "2".to_string(),
                target,
            },
            rope_id,
            PreSpawned::new(super::super::rope::rope_link_hash(rope_id)).for_receiver(client_link),
        ))
        .id()
}

#[test]
fn old_attached_confirmation_replayed_through_release_ends_detached_before_presentation() {
    const SERVER_CONFIRM_TICK: u16 = 98;
    const CLIENT_RELEASE_TICK: u16 = 100;

    let mut app = rope_test_app();
    let client_link = app.world_mut().spawn(ClientSpawned).id();
    spawn_predicted_local_player_and_box(&mut app);
    app.add_systems(FixedUpdate, super::super::rope::toggle_rope);

    spawn_client_rope(&mut app, client_link);
    assert_eq!(
        active_visible_rope_count(&mut app, "2"),
        1,
        "test setup: rope is predicted attached at tick {SERVER_CONFIRM_TICK}"
    );

    release_rope_toggle(&mut app);
    run_rope_frame(&mut app);
    assert_eq!(
        active_visible_rope_count(&mut app, "2"),
        0,
        "client release at tick {CLIENT_RELEASE_TICK} must detach locally"
    );

    // Unit-level model of Lightyear restoring an old confirmed tick during
    // rollback: the server says the rope was attached at tick 98, then the
    // fixed-sim logic is replayed through the release input at tick 100 before
    // presentation. The production UDP regression covers the real rollback and
    // input-buffer pipeline; this test boxes in the rope decision logic.
    spawn_client_rope(&mut app, client_link);
    release_rope_toggle(&mut app);
    run_rope_frame(&mut app);

    assert_eq!(
        active_visible_rope_count(&mut app, "2"),
        0,
        "confirmed attached state from tick {SERVER_CONFIRM_TICK} must not survive replay through release tick {CLIENT_RELEASE_TICK}"
    );
}

#[test]
fn client_release_while_moving_away_hides_rope_immediately() {
    let mut app = rope_test_app();
    app.init_resource::<super::super::rope::LocallyReleasedRopes>();
    let client_link = app.world_mut().spawn(ClientSpawned).id();
    let (player, _) = spawn_predicted_local_player_and_box(&mut app);
    spawn_client_rope(&mut app, client_link);

    app.world_mut()
        .entity_mut(player)
        .get_mut::<LinearVelocity>()
        .expect("player has velocity")
        .0 = Vec3::NEG_X * 5.0;
    app.add_systems(
        FixedUpdate,
        (
            super::super::rope::toggle_rope,
            super::super::rope::suppress_locally_released_rope_reappearances,
        )
            .chain(),
    );

    release_rope_toggle(&mut app);
    run_rope_frame(&mut app);

    assert_eq!(
        active_visible_rope_count(&mut app, "2"),
        0,
        "local release must hide the rope on the same simulated frame"
    );
    assert!(
        app.world()
            .resource::<super::super::rope::LocallyReleasedRopes>()
            .contains(super::super::rope::rope_id_for_input("2", test_box_id(0))),
        "client should remember the locally released deterministic rope id"
    );
}

#[test]
fn repeated_release_while_local_rope_is_disabled_does_not_spawn_duplicate_hashes() {
    let mut app = rope_test_app();
    app.init_resource::<super::super::rope::LocallyReleasedRopes>();
    let client_link = app.world_mut().spawn(ClientSpawned).id();
    spawn_predicted_local_player_and_box(&mut app);

    let target = test_box_id(0);
    let rope_id = super::super::rope::rope_id_for_input("2", target);
    app.world_mut()
        .resource_mut::<super::super::rope::LocallyReleasedRopes>()
        .suppress(rope_id);
    app.world_mut().spawn((
        RopeLink {
            rope_id,
            player_owner: "2".to_string(),
            target,
        },
        rope_id,
        PreSpawned::new(super::super::rope::rope_link_hash(rope_id)).for_receiver(client_link),
        PredictionDisable,
        super::super::rope::RopeJointDetachPending,
    ));

    app.add_systems(
        FixedUpdate,
        (
            super::super::rope::toggle_rope,
            super::super::rope::suppress_locally_released_rope_reappearances,
        )
            .chain(),
    );

    release_rope_toggle(&mut app);
    run_rope_frame(&mut app);

    let matching_links = app
        .world_mut()
        .query::<(&RopeLink, &PreSpawned)>()
        .iter(app.world())
        .filter(|(link, prespawned)| {
            link.rope_id == rope_id
                && prespawned.hash == Some(super::super::rope::rope_link_hash(rope_id))
        })
        .count();
    assert!(
        matching_links <= 1,
        "a replayed/rebroadcast release must not spawn another PreSpawned rope with the same hash"
    );
    assert_eq!(active_visible_rope_count(&mut app, "2"), 0);
}

#[test]
fn delayed_release_edge_cleans_up_already_hidden_local_rope() {
    let mut app = rope_test_app();
    app.init_resource::<super::super::rope::LocallyReleasedRopes>();
    let client_link = app.world_mut().spawn(ClientSpawned).id();
    spawn_predicted_local_player_and_box(&mut app);

    let target = test_box_id(0);
    let rope_id = super::super::rope::rope_id_for_input("2", target);
    let rope = spawn_client_rope(&mut app, client_link);
    app.world_mut()
        .resource_mut::<super::super::rope::LocallyReleasedRopes>()
        .suppress(rope_id);
    app.world_mut().entity_mut(rope).insert(PredictionDisable);

    app.add_systems(FixedUpdate, super::super::rope::toggle_rope);
    release_rope_toggle(&mut app);
    run_rope_frame(&mut app);

    let cleaned_up = app.world().get::<RopeLink>(rope).is_none()
        || app
            .world()
            .get::<super::super::rope::RopeJointDetachPending>(rope)
            .is_some();
    assert!(
        cleaned_up,
        "the replayed ActionState release must still route through detach cleanup after the physical hider hid the rope"
    );
    assert_eq!(active_visible_rope_count(&mut app, "2"), 0);
}

#[test]
fn locally_released_rope_reappearance_is_suppressed_repeatedly() {
    let mut app = rope_test_app();
    app.init_resource::<super::super::rope::LocallyReleasedRopes>();
    let client_link = app.world_mut().spawn(ClientSpawned).id();
    let (player, _) = spawn_predicted_local_player_and_box(&mut app);
    let rope_id = super::super::rope::rope_id_for_input("2", test_box_id(0));
    app.world_mut()
        .resource_mut::<super::super::rope::LocallyReleasedRopes>()
        .suppress(rope_id);

    app.add_systems(
        FixedUpdate,
        super::super::rope::suppress_locally_released_rope_reappearances,
    );

    let stale = spawn_client_rope(&mut app, client_link);
    for step in 0..6 {
        app.world_mut()
            .entity_mut(player)
            .get_mut::<Transform>()
            .expect("player has transform")
            .translation += Vec3::NEG_X;
        app.world_mut()
            .entity_mut(stale)
            .remove::<PredictionDisable>()
            .remove::<super::super::rope::RopeJointDetachPending>();

        run_rope_frame(&mut app);

        assert_eq!(
            active_visible_rope_count(&mut app, "2"),
            0,
            "stale confirmed rope re-enable at step {step} must be hidden again"
        );
        assert!(app.world().get::<PredictionDisable>(stale).is_some());
        assert!(
            app.world()
                .get::<super::super::rope::RopeJointDetachPending>(stale)
                .is_some()
        );
    }
}

#[test]
fn locally_released_rope_suppression_removes_stale_joint() {
    let mut app = rope_test_app();
    app.init_resource::<super::super::rope::LocallyReleasedRopes>();
    app.world_mut().spawn(ClientSpawned);
    spawn_predicted_local_player_and_box(&mut app);

    let target = test_box_id(0);
    let rope_id = super::super::rope::rope_id_for_input("2", target);
    app.world_mut()
        .resource_mut::<super::super::rope::LocallyReleasedRopes>()
        .suppress(rope_id);
    let joint = app.world_mut().spawn(RopeJoint).id();
    let stale = app
        .world_mut()
        .spawn((
            RopeLink {
                rope_id,
                player_owner: "2".to_string(),
                target,
            },
            super::super::rope::RopeJointEntity(joint),
        ))
        .id();

    app.add_systems(
        FixedUpdate,
        super::super::rope::suppress_locally_released_rope_reappearances,
    );
    run_rope_frame(&mut app);

    assert!(app.world().get::<RopeJoint>(joint).is_none());
    assert!(
        app.world()
            .get::<super::super::rope::RopeJointEntity>(stale)
            .is_none()
    );
    assert_eq!(active_visible_rope_count(&mut app, "2"), 0);
}

#[test]
fn explicit_reattach_allows_same_deterministic_rope_after_release() {
    let mut app = rope_test_app();
    app.init_resource::<super::super::rope::LocallyReleasedRopes>();
    let client_link = app.world_mut().spawn(ClientSpawned).id();
    spawn_predicted_local_player_and_box(&mut app);
    let first = spawn_client_rope(&mut app, client_link);

    app.add_systems(
        FixedUpdate,
        (
            super::super::rope::toggle_rope,
            super::super::rope::suppress_locally_released_rope_reappearances,
        )
            .chain(),
    );

    release_rope_toggle(&mut app);
    run_rope_frame(&mut app);
    assert_eq!(active_visible_rope_count(&mut app, "2"), 0);
    // Emulate authoritative removal of the first rope without directly
    // despawning a Lightyear-tracked PreSpawned entity from the test.
    if let Ok(mut entity) = app.world_mut().get_entity_mut(first) {
        entity.remove::<(
            RopeLink,
            StableEntityId,
            PreSpawned,
            PredictionDisable,
            super::super::rope::RopeJointDetachPending,
        )>();
    }

    release_rope_toggle(&mut app);
    run_rope_frame(&mut app);

    let expected_rope_id = super::super::rope::rope_id_for_input("2", test_box_id(0));
    let active: Vec<_> = app
        .world_mut()
        .query::<(
            &RopeLink,
            Option<&super::super::rope::RopeJointDetachPending>,
            Option<&PredictionDisable>,
        )>()
        .iter(app.world())
        .filter(|(_, pending, disabled)| pending.is_none() && disabled.is_none())
        .map(|(link, _, _)| link.rope_id)
        .collect();

    assert_eq!(active, vec![expected_rope_id]);
    assert!(
        !app.world()
            .resource::<super::super::rope::LocallyReleasedRopes>()
            .contains(expected_rope_id)
    );
}
