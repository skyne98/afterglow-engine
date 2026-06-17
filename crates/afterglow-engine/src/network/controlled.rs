//! Controlled-entity lifecycle orchestration.
//!
//! Provides the reusable layer that games need: tracking which server-spawned
//! entities are controlled by which client links, and binding [`ControlledBy`]
//! automatically when a `ClientOf` link appears.
//!
//! Games are responsible for:
//! - Spawning the entity on `SessionEvent::MemberJoined`
//! - Tagging it with a component that identifies the owning member
//! - Despawning on `SessionEvent::MemberLeft`
//!
//! The engine handles:
//! - Finding the `ClientOf` link entity for a given member
//! - Inserting [`ControlledBy`] on the entity once the link exists
//! - Exposing a [`ControlledEntityRegistry`] for lookup

use bevy::prelude::*;
use lightyear::prelude::{server::ClientOf, *};

/// Marker trait for components that identify which member owns an entity.
/// Games implement this on their own player/character component.
pub trait OwnershipSource: Component {
    /// Returns the Lightyear `PeerId` that owns this entity, if any.
    fn owning_peer(&self) -> Option<PeerId>;
}

/// Registry mapping session members to their controlled entities.
#[derive(Resource, Default, Debug, Clone)]
pub struct ControlledEntityRegistry {
    /// Maps a peer ID to the entity they control.
    pub entities: bevy::platform::collections::HashMap<PeerId, Entity>,
}

impl ControlledEntityRegistry {
    /// Returns the entity controlled by the given peer, if any.
    pub fn entity_for(&self, peer: PeerId) -> Option<Entity> {
        self.entities.get(&peer).copied()
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
/// identified by a numeric peer ID stored as a string.
///
/// Games that store the owner as a string (e.g. a member ID rendered as a
/// string) can use this directly by adding the [`PlayerOwned`] component to
/// their entities.
#[derive(Component, Clone, Debug, Default, Reflect)]
#[reflect(Component)]
pub struct PlayerOwned {
    /// The owning peer's ID, parsed from the owner string.
    pub peer: Option<PeerId>,
}

impl OwnershipSource for PlayerOwned {
    fn owning_peer(&self) -> Option<PeerId> {
        self.peer
    }
}

impl PlayerOwned {
    /// Creates a `PlayerOwned` from an owner string that parses to a `u64`.
    pub fn from_owner_str(owner: &str) -> Self {
        Self {
            peer: owner.parse::<u64>().ok().map(PeerId::Netcode),
        }
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
    players: Query<(Entity, &O), (With<Replicate>, Without<ControlledBy>)>,
    links: Query<(Entity, &RemoteId), With<ClientOf>>,
) {
    for (player_entity, owner_source) in &players {
        let Some(peer) = owner_source.owning_peer() else {
            continue;
        };
        let Some((link_entity, _)) = links.iter().find(|(_, remote)| remote.0 == peer) else {
            continue;
        };
        commands.entity(player_entity).insert(ControlledBy {
            owner: link_entity,
            lifetime: Default::default(),
        });
    }
}
