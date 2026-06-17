use std::time::Duration;

use lightyear::prelude::server::ClientOf;

use super::*;
use crate::network::session::SessionMemberId;

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<MemberLinkMap>();
    // Set up minimal Lightyear replication infrastructure so `Replicate`
    // component doesn't panic.
    app.add_plugins(lightyear::prelude::server::ServerPlugins {
        tick_duration: Duration::from_secs_f64(1.0 / 60.0),
    });
    app
}

fn ready_link(member: SessionMemberId) -> impl Bundle {
    (
        ClientOf,
        RemoteId(PeerId::Netcode(member.as_raw() as u64)),
        ReplicationSender::new(
            Duration::from_millis(16),
            SendUpdatesMode::SinceLastAck,
            false,
        ),
    )
}

#[test]
fn player_owned_from_member_stores_member() {
    let member = SessionMemberId::new(42);
    let owned = PlayerOwned::from_member(member);
    assert_eq!(owned.owning_member(), Some(member));
}

#[test]
fn player_owned_invalid_member_returns_none() {
    let owned = PlayerOwned::from_member(SessionMemberId::INVALID);
    assert_eq!(owned.owning_member(), None);
}

#[test]
fn member_link_map_starts_empty() {
    let map = MemberLinkMap::default();
    assert!(map.link_for(SessionMemberId::new(1)).is_none());
}

#[test]
fn update_member_link_map_adds_netcode_links() {
    let mut app = test_app();
    let member = SessionMemberId::new(7);
    let link_entity = app.world_mut().spawn(ready_link(member)).id();

    app.add_systems(Update, update_member_link_map);
    app.update();

    let map = app.world().resource::<MemberLinkMap>();
    assert_eq!(map.link_for(member), Some(link_entity));
}

#[test]
fn update_member_link_map_waits_for_replication_sender() {
    let mut app = test_app();
    let member = SessionMemberId::new(8);
    app.world_mut()
        .spawn((ClientOf, RemoteId(PeerId::Netcode(member.as_raw() as u64))));

    app.add_systems(Update, update_member_link_map);
    app.update();

    let map = app.world().resource::<MemberLinkMap>();
    assert!(
        map.link_for(member).is_none(),
        "ClientOf without ReplicationSender must not be bindable by ControlledBy"
    );
}

#[test]
fn update_member_link_map_ignores_local_peers() {
    let mut app = test_app();
    app.world_mut()
        .spawn((ClientOf, RemoteId(PeerId::Local(1))));

    app.add_systems(Update, update_member_link_map);
    app.update();

    let map = app.world().resource::<MemberLinkMap>();
    assert!(map.links.is_empty());
}

#[test]
fn update_member_link_map_removes_stale_entries() {
    let mut app = test_app();
    let member = SessionMemberId::new(5);
    let link_entity = app.world_mut().spawn(ready_link(member)).id();

    app.add_systems(Update, update_member_link_map);
    app.update();

    // Remove the link entity
    app.world_mut().entity_mut(link_entity).despawn();
    app.update();

    let map = app.world().resource::<MemberLinkMap>();
    assert!(map.link_for(member).is_none());
}

#[test]
fn bind_controlled_entities_inserts_controlled_by() {
    let mut app = test_app();
    let member = SessionMemberId::new(3);
    let link_entity = app.world_mut().spawn(ready_link(member)).id();
    app.update(); // run update_member_link_map to populate the map

    let player_entity = app
        .world_mut()
        .spawn((
            PlayerOwned::from_member(member),
            Replicate::to_clients(NetworkTarget::All),
        ))
        .id();

    app.add_systems(
        Update,
        (
            update_member_link_map,
            bind_controlled_entities::<PlayerOwned>,
        )
            .chain(),
    );
    app.update();

    let controlled_by = app
        .world()
        .get::<ControlledBy>(player_entity)
        .expect("ControlledBy should be inserted");
    assert_eq!(controlled_by.owner, link_entity);
}

#[test]
fn bind_controlled_entities_skips_without_link() {
    let mut app = test_app();
    let member = SessionMemberId::new(99);
    // No ClientOf link spawned for this member
    let player_entity = app
        .world_mut()
        .spawn((
            PlayerOwned::from_member(member),
            Replicate::to_clients(NetworkTarget::All),
        ))
        .id();

    app.add_systems(Update, bind_controlled_entities::<PlayerOwned>);
    app.update();

    assert!(app.world().get::<ControlledBy>(player_entity).is_none());
}

#[test]
fn bind_controlled_entities_is_idempotent() {
    let mut app = test_app();
    let member = SessionMemberId::new(11);
    app.world_mut().spawn(ready_link(member));
    app.update();

    let player_entity = app
        .world_mut()
        .spawn((
            PlayerOwned::from_member(member),
            Replicate::to_clients(NetworkTarget::All),
        ))
        .id();

    app.add_systems(
        Update,
        (
            update_member_link_map,
            bind_controlled_entities::<PlayerOwned>,
        )
            .chain(),
    );
    app.update();
    app.update(); // run again

    // Should still have exactly one ControlledBy
    assert!(app.world().get::<ControlledBy>(player_entity).is_some());
}

#[test]
fn bind_controlled_entities_skips_invalid_member() {
    let mut app = test_app();
    let player_entity = app
        .world_mut()
        .spawn((
            PlayerOwned::from_member(SessionMemberId::INVALID),
            Replicate::to_clients(NetworkTarget::All),
        ))
        .id();

    app.add_systems(Update, bind_controlled_entities::<PlayerOwned>);
    app.update();

    assert!(app.world().get::<ControlledBy>(player_entity).is_none());
}
