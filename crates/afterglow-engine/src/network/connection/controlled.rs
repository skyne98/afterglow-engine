//! Controlled-entity lifecycle with [`PlayerId`] (u64) keying.
//!
//! This module replaces the deleted legacy `crate::network::controlled` code.
//!
//! Key differences:
//! - [`MemberLinkMap`] is keyed by [`PlayerId`] (u64) instead of
//!   `SessionMemberId` (u128).
//! - [`PlayerOwned`] stores a `PlayerId` instead of `SessionMemberId`.
//! - Direct/no-auth links enter `MemberLinkMap` on `ClientOf`; auth-required
//!   links enter it only after challenge-response succeeds.
//! - `bind_controlled_entities` uses [`PlayerOwned`]'s `PlayerId` to look up
//!   the link entity.

use bevy::prelude::*;
use lightyear::prelude::{server::ClientOf, *};

use super::PlayerId;

/// Registry mapping [`PlayerId`]s to their Lightyear client link entities.
///
/// Populated by the `On<Add, ClientOf>` observer in
/// [`super::auth::on_client_of_added`] when new connections are established.
/// Entries are removed when the link entity is despawned (observed via
/// `On<Remove, ClientOf>` or polling).
#[derive(Resource, Default, Debug, Clone)]
pub struct MemberLinkMap {
    pub links: bevy::platform::collections::HashMap<PlayerId, Entity>,
}

impl MemberLinkMap {
    /// Returns the link entity for the given player, if any.
    pub fn link_for(&self, player_id: PlayerId) -> Option<Entity> {
        self.links.get(&player_id).copied()
    }
}

/// Marker trait for components that identify which player owns an entity.
///
/// Games implement this on their own player/character component.
pub trait PlayerOwnershipSource: Component {
    fn owning_player(&self) -> Option<PlayerId>;
}

/// A blanket [`PlayerOwnershipSource`] implementation for entities whose
/// owner is identified by a [`PlayerId`].
///
/// Games can use this directly by adding the [`PlayerOwned`] component to
/// their entities.
#[derive(Component, Clone, Debug, Default, Reflect)]
#[reflect(Component)]
pub struct PlayerOwned {
    pub player_id: PlayerId,
}

impl PlayerOwnershipSource for PlayerOwned {
    fn owning_player(&self) -> Option<PlayerId> {
        if self.player_id != 0 {
            Some(self.player_id)
        } else {
            None
        }
    }
}

impl PlayerOwned {
    pub fn from_player_id(player_id: PlayerId) -> Self {
        Self { player_id }
    }
}

/// Plugin that adds controlled-entity lifecycle systems.
///
/// Games add this plugin, implement [`PlayerOwnershipSource`] on their player
/// component, and spawn/despawn entities on [`super::ConnectionEvent`]. The
/// engine handles `ControlledBy` binding.
pub struct ControlledEntityPlugin;

impl Plugin for ControlledEntityPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            (
                remove_stale_member_links,
                bind_controlled_entities::<PlayerOwned>,
            )
                .chain(),
        );
    }
}

/// Removes stale entries from [`MemberLinkMap`] when a link entity is
/// despawned.
fn remove_stale_member_links(
    mut member_links: ResMut<MemberLinkMap>,
    links: Query<(), With<ClientOf>>,
) {
    member_links
        .links
        .retain(|_, entity| links.get(*entity).is_ok());
}

/// System that binds [`ControlledBy`] on entities that have a
/// [`PlayerOwnershipSource`] but no `ControlledBy` yet.
///
/// Runs every frame and is idempotent.
pub fn bind_controlled_entities<O: PlayerOwnershipSource>(
    mut commands: Commands,
    member_links: Res<MemberLinkMap>,
    client_links: Query<(), (With<ClientOf>, With<ReplicationSender>)>,
    players: Query<(Entity, &O), (With<Replicate>, Without<ControlledBy>)>,
) {
    for (player_entity, owner_source) in &players {
        let Some(player_id) = owner_source.owning_player() else {
            continue;
        };
        let Some(link_entity) = member_links.link_for(player_id) else {
            continue;
        };
        if client_links.get(link_entity).is_err() {
            continue;
        }
        commands.entity(player_entity).insert(ControlledBy {
            owner: link_entity,
            lifetime: Default::default(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn replication_sender() -> ReplicationSender {
        ReplicationSender::new(
            Duration::from_millis(16),
            SendUpdatesMode::SinceLastAck,
            false,
        )
    }

    #[test]
    fn bind_controlled_entities_waits_for_replication_sender() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(lightyear::prelude::server::ServerPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / 60.0),
        });
        app.init_resource::<MemberLinkMap>();
        app.add_systems(Update, bind_controlled_entities::<PlayerOwned>);
        app.finish();
        app.cleanup();

        let link = app.world_mut().spawn(ClientOf).id();
        app.world_mut()
            .resource_mut::<MemberLinkMap>()
            .links
            .insert(7, link);
        let player = app
            .world_mut()
            .spawn((
                PlayerOwned::from_player_id(7),
                Replicate::to_clients(NetworkTarget::All),
            ))
            .id();

        app.update();
        assert!(!app.world().entity(player).contains::<ControlledBy>());

        app.world_mut()
            .entity_mut(link)
            .insert(replication_sender());
        app.update();
        assert!(app.world().entity(player).contains::<ControlledBy>());
    }

    #[test]
    fn remove_stale_member_links_keeps_clientof_before_sender_is_ready() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<MemberLinkMap>();
        app.add_systems(Update, remove_stale_member_links);

        let link = app.world_mut().spawn(ClientOf).id();
        app.world_mut()
            .resource_mut::<MemberLinkMap>()
            .links
            .insert(9, link);

        app.update();
        assert_eq!(
            app.world().resource::<MemberLinkMap>().link_for(9),
            Some(link)
        );

        app.world_mut().entity_mut(link).despawn();
        app.update();
        assert_eq!(app.world().resource::<MemberLinkMap>().link_for(9), None);
    }
}
