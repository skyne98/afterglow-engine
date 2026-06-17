//! Controlled-entity lifecycle orchestration.
//!
//! Provides the reusable layer that games need: binding [`ControlledBy`] on
//! server-spawned entities to the correct client link, using stable
//! [`SessionMemberId`] identity rather than ephemeral `PeerId`s.
//!
//! Games are responsible for:
//! - Spawning the entity on `SessionEvent::MemberJoined`
//! - Tagging it with a component that identifies the owning member
//! - Despawning on `SessionEvent::MemberLeft`
//!
//! The engine handles:
//! - Finding the `ClientOf` link entity for a given `SessionMemberId` (via
//!   [`MemberLinkMap`], populated by the Lightyear bridge)
//! - Inserting [`ControlledBy`] on the entity once the link exists

use bevy::prelude::*;
use lightyear::prelude::{server::ClientOf, *};

use crate::network::session::SessionMemberId;

/// Marker trait for components that identify which session member owns an
/// entity. Games implement this on their own player/character component.
pub trait OwnershipSource: Component {
    /// Returns the session member that owns this entity, if any.
    fn owning_member(&self) -> Option<SessionMemberId>;
}

/// Registry mapping session members to their Lightyear client link entities.
///
/// Populated by the Lightyear bridge when `ClientOf` links are created or
/// removed. Used by [`bind_controlled_entities`] to find the correct link
/// entity for a given `SessionMemberId`.
///
/// The mapping from `SessionMemberId` to `PeerId` is established by the
/// bridge: `SessionMemberId.as_raw()` is used as the netcode `client_id`,
/// which becomes `PeerId::Netcode(client_id)` on the server.
#[derive(Resource, Default, Debug, Clone)]
pub struct MemberLinkMap {
    /// Maps a session member ID to the `ClientOf` link entity.
    pub links: bevy::platform::collections::HashMap<SessionMemberId, Entity>,
}

impl MemberLinkMap {
    /// Returns the link entity for the given member, if any.
    pub fn link_for(&self, member: SessionMemberId) -> Option<Entity> {
        self.links.get(&member).copied()
    }
}

/// Plugin that adds the controlled-entity lifecycle systems.
///
/// Games add this plugin, implement [`OwnershipSource`] on their player
/// component, and spawn/despawn entities on session events. The engine handles
/// `ControlledBy` binding.
pub struct ControlledEntityPlugin;

impl Plugin for ControlledEntityPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, bind_controlled_entities::<PlayerOwned>);
    }
}

/// A blanket [`OwnershipSource`] implementation for entities whose owner is
/// identified by a [`SessionMemberId`].
///
/// Games that store the owner as a `SessionMemberId` can use this directly by
/// adding the [`PlayerOwned`] component to their entities.
#[derive(Component, Clone, Debug, Default, Reflect)]
#[reflect(Component)]
pub struct PlayerOwned {
    pub member: SessionMemberId,
}

impl OwnershipSource for PlayerOwned {
    fn owning_member(&self) -> Option<SessionMemberId> {
        if self.member.is_valid() {
            Some(self.member)
        } else {
            None
        }
    }
}

impl PlayerOwned {
    /// Creates a `PlayerOwned` from a `SessionMemberId`.
    pub fn from_member(member: SessionMemberId) -> Self {
        Self { member }
    }
}

/// System that binds [`ControlledBy`] on entities that have an
/// [`OwnershipSource`] but no `ControlledBy` yet. Runs every frame; is
/// idempotent.
///
/// Games can provide their own `OwnershipSource` implementation and add a
/// system with their specific component type:
/// ```ignore
/// app.add_systems(Update, bind_controlled_entities::<MyPlayerComponent>);
/// ```
pub fn bind_controlled_entities<O: OwnershipSource>(
    mut commands: Commands,
    member_links: Res<MemberLinkMap>,
    players: Query<(Entity, &O), (With<Replicate>, Without<ControlledBy>)>,
) {
    for (player_entity, owner_source) in &players {
        let Some(member) = owner_source.owning_member() else {
            continue;
        };
        let Some(link_entity) = member_links.link_for(member) else {
            continue;
        };
        commands.entity(player_entity).insert(ControlledBy {
            owner: link_entity,
            lifetime: Default::default(),
        });
    }
}

/// Updates [`MemberLinkMap`] by scanning `ClientOf` link entities and
/// extracting their `SessionMemberId` from the `RemoteId`.
///
/// The bridge assigns `SessionMemberId.as_raw()` as the netcode `client_id`,
/// so on the server, a `ClientOf` link's `RemoteId(PeerId::Netcode(id))`
/// corresponds to `SessionMemberId::new(id as u128)`.
///
/// This system is added by the Lightyear bridge plugin, not by
/// [`ControlledEntityPlugin`], so it runs even if the game doesn't use
/// `ControlledEntityPlugin`.
pub fn update_member_link_map(
    mut member_links: ResMut<MemberLinkMap>,
    links: Query<(Entity, &RemoteId), With<ClientOf>>,
) {
    // Remove stale entries (link entity no longer exists)
    member_links.links.retain(|_, &mut entity| {
        // If the entity is still a ClientOf link, keep it
        links.get(entity).is_ok()
    });

    // Add new entries
    for (entity, remote_id) in &links {
        if let PeerId::Netcode(id) = remote_id.0 {
            let member = SessionMemberId::new(id as u128);
            member_links.links.insert(member, entity);
        }
    }
}
