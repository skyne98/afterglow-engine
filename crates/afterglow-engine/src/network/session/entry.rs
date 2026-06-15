use std::collections::HashMap;

use super::{PlayerIdentity, SessionCode, SessionConfig, SessionId, SessionInfo, SessionMemberId};

#[derive(Clone, Debug)]
pub(crate) struct SessionEntry {
    pub(crate) config: SessionConfig,
    pub(crate) code: SessionCode,
    pub(crate) owner: SessionMemberId,
    pub(crate) owner_identity: PlayerIdentity,
    pub(crate) members: Vec<SessionMemberId>,
    /// Identity bound to each member slot. Used for rejoin detection and for
    /// the Steam backend to validate tickets later.
    pub(crate) member_identities: HashMap<SessionMemberId, PlayerIdentity>,
    /// Native public key -> member id. Kept separate so rejoin is O(1).
    pub(crate) key_to_member: HashMap<[u8; 32], SessionMemberId>,
}

impl SessionEntry {
    pub(crate) fn to_info(&self, id: SessionId) -> SessionInfo {
        SessionInfo {
            id,
            code: self.code.clone(),
            backend: self.config.backend,
            name: self.config.name.clone(),
            owner: self.owner,
            owner_identity: self.owner_identity.clone(),
            member_count: self.members.len() as u32,
            max_members: self.config.max_members,
            visibility: self.config.visibility,
            metadata: self.config.metadata.clone(),
            transport: self.config.transport.clone(),
        }
    }
}

#[allow(clippy::collapsible_if)]
pub(crate) fn insert_native_key(
    entry: &mut SessionEntry,
    identity: &PlayerIdentity,
    member_id: SessionMemberId,
) {
    if let PlayerIdentity::Native(proof) = identity {
        if let Ok(key) = <[u8; 32]>::try_from(proof.public_key.as_slice()) {
            // Guard against accidental key reuse by a different member. A
            // colliding key that already maps to a *different* member is a
            // bug or an attack — the new member must rejoin under their
            // existing member id instead. Re-inserting the same key for the
            // same member is a no-op and is allowed.
            if let Some(existing) = entry.key_to_member.get(&key) {
                debug_assert_eq!(
                    *existing, member_id,
                    "native public key collision: key already maps to a different member"
                );
                return;
            }
            entry.key_to_member.insert(key, member_id);
        }
    }
}

#[allow(clippy::collapsible_if)]
pub(crate) fn remove_native_key(entry: &mut SessionEntry, identity: &PlayerIdentity) {
    if let PlayerIdentity::Native(proof) = identity {
        if let Ok(key) = <[u8; 32]>::try_from(proof.public_key.as_slice()) {
            entry.key_to_member.remove(&key);
        }
    }
}
