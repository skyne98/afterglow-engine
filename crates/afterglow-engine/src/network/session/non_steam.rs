use bevy::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};

use super::{
    AfterglowSessionState, PlayerIdentity, SessionBackend, SessionCode, SessionConfig,
    SessionError, SessionEvent, SessionId, SessionIdentityNonce, SessionInfo, SessionLeaveReason,
    SessionMemberId, SessionRequest, SessionSearch,
    entry::{SessionEntry, insert_native_key, remove_native_key},
};

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------

#[derive(Resource, Debug)]
pub(crate) struct NonSteamSessionCatalog {
    pub(crate) next_id: u128,
    pub(crate) next_member_id: u128,
    pub(crate) next_code_seed: u64,
    pub(crate) sessions: BTreeMap<SessionId, SessionEntry>,
    pub(crate) used_codes: HashSet<SessionCode>,
}

impl Default for NonSteamSessionCatalog {
    fn default() -> Self {
        Self {
            next_id: 1,
            next_member_id: 1,
            next_code_seed: 1,
            sessions: BTreeMap::new(),
            used_codes: HashSet::new(),
        }
    }
}

impl NonSteamSessionCatalog {
    pub(crate) fn allocate_member_id(&mut self) -> SessionMemberId {
        let id = SessionMemberId(self.next_member_id);
        self.next_member_id += 1;
        id
    }

    /// Allocate a unique short session code.
    ///
    /// In the astronomically unlikely case the generated code collides with an
    /// active session, the seed is advanced until a free code is found.
    pub(crate) fn allocate_code(&mut self) -> SessionCode {
        loop {
            let candidate = SessionCode::generate(self.next_code_seed);
            self.next_code_seed += 1;
            if !self.used_codes.contains(&candidate) {
                self.used_codes.insert(candidate.clone());
                return candidate;
            }
        }
    }
}

#[cfg(test)]
impl NonSteamSessionCatalog {
    pub(crate) fn seed_session(
        &mut self,
        config: SessionConfig,
        owner: SessionMemberId,
        owner_identity: PlayerIdentity,
    ) -> SessionId {
        let id = SessionId(self.next_id);
        self.next_id += 1;
        let code = self.allocate_code();
        let mut entry = SessionEntry {
            config,
            code,
            owner,
            owner_identity: owner_identity.clone(),
            members: vec![owner],
            member_identities: HashMap::new(),
            key_to_member: HashMap::new(),
        };
        entry
            .member_identities
            .insert(owner, owner_identity.clone());
        insert_native_key(&mut entry, &owner_identity, owner);
        self.sessions.insert(id, entry);
        id
    }

    pub(crate) fn add_member(
        &mut self,
        session: SessionId,
        member: SessionMemberId,
        identity: PlayerIdentity,
    ) {
        if let Some(entry) = self.sessions.get_mut(&session) {
            entry.members.push(member);
            entry.member_identities.insert(member, identity.clone());
            insert_native_key(entry, &identity, member);
        }
    }

    pub(crate) fn remove_session(&mut self, session: SessionId) {
        self.sessions.remove(&session);
    }
}

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------

pub(crate) fn process_non_steam_session_requests(
    mut catalog: ResMut<NonSteamSessionCatalog>,
    mut state: ResMut<AfterglowSessionState>,
    nonce: Res<SessionIdentityNonce>,
    mut requests: MessageReader<SessionRequest>,
    mut messages: MessageWriter<SessionEvent>,
) {
    for request in requests.read() {
        let mut events = Vec::new();
        match request {
            SessionRequest::Create(config, identity) => {
                if !is_non_steam_backend(&config.backend, &mut events) {
                    for event in events {
                        messages.write(event);
                    }
                    continue;
                }
                handle_create(
                    config,
                    identity,
                    &nonce.0,
                    &mut catalog,
                    &mut state,
                    &mut events,
                );
            }
            SessionRequest::Search(search) => {
                if !is_non_steam_backend(&search.backend, &mut events) {
                    for event in events {
                        messages.write(event);
                    }
                    continue;
                }
                handle_search(search, &catalog, &mut events);
            }
            SessionRequest::Join {
                backend,
                session,
                identity,
                ..
            } => {
                if !is_non_steam_backend(backend, &mut events) {
                    for event in events {
                        messages.write(event);
                    }
                    continue;
                }
                let target = session.as_raw().to_string();
                handle_join(
                    *session,
                    &target,
                    identity,
                    &nonce.0,
                    &mut catalog,
                    &mut state,
                    &mut events,
                );
            }
            SessionRequest::JoinByCode {
                backend,
                code,
                identity,
                ..
            } => {
                if !is_non_steam_backend(backend, &mut events) {
                    for event in events {
                        messages.write(event);
                    }
                    continue;
                }
                handle_join_by_code(
                    code.clone(),
                    identity,
                    &nonce.0,
                    &mut catalog,
                    &mut state,
                    &mut events,
                );
            }
            SessionRequest::Leave => match state.current_backend {
                Some(SessionBackend::Steam) => {
                    events.push(SessionEvent::Error(SessionError::BackendUnavailable));
                }
                Some(SessionBackend::NonSteam) | None => {
                    handle_leave(&mut catalog, &mut state, &mut events);
                }
            },
        }
        for event in events {
            messages.write(event);
        }
    }
}

fn is_non_steam_backend(backend: &SessionBackend, events: &mut Vec<SessionEvent>) -> bool {
    match backend {
        SessionBackend::NonSteam => true,
        SessionBackend::Steam => {
            events.push(SessionEvent::Error(SessionError::BackendUnavailable));
            false
        }
    }
}

/// Validate an identity proof for the NonSteam backend.
///
/// Native Ed25519 proofs are verified against the canonical challenge message.
/// Steam proofs are accepted as passthrough — the Steam provider will validate
/// the ticket when `SessionBackend::Steam` is selected.
pub(crate) fn validate_identity_for_non_steam(
    identity: &PlayerIdentity,
    backend: SessionBackend,
    target: &str,
    nonce: &[u8; 32],
) -> Result<(), super::IdentityError> {
    match identity {
        PlayerIdentity::Native(proof) => proof.verify(backend, target, nonce),
        PlayerIdentity::Steam { .. } => Ok(()),
    }
}

pub(crate) fn handle_create(
    config: &SessionConfig,
    identity: &PlayerIdentity,
    nonce: &[u8; 32],
    catalog: &mut NonSteamSessionCatalog,
    state: &mut AfterglowSessionState,
    events: &mut Vec<SessionEvent>,
) {
    if state.current_session.is_some() {
        events.push(SessionEvent::Error(SessionError::AlreadyInSession));
        return;
    }
    if config.max_members == 0 {
        events.push(SessionEvent::Error(SessionError::InvalidConfig));
        return;
    }

    if validate_identity_for_non_steam(identity, config.backend, "create", nonce).is_err() {
        events.push(SessionEvent::Error(SessionError::PermissionDenied));
        return;
    }

    let id = SessionId(catalog.next_id);
    catalog.next_id += 1;
    let code = catalog.allocate_code();
    let owner = ensure_local_member_id(catalog, state);

    let mut entry = SessionEntry {
        config: config.clone(),
        code,
        owner,
        owner_identity: identity.clone(),
        members: vec![owner],
        member_identities: HashMap::new(),
        key_to_member: HashMap::new(),
    };
    entry.member_identities.insert(owner, identity.clone());
    insert_native_key(&mut entry, identity, owner);
    catalog.sessions.insert(id, entry);

    state.current_session = Some(id);
    state.current_backend = Some(SessionBackend::NonSteam);
    state.identity = Some(identity.clone());

    let info = catalog.sessions[&id].to_info(id);
    events.push(SessionEvent::Created(info.clone()));
    events.push(SessionEvent::Joined(info));
}

pub(crate) fn handle_search(
    search: &SessionSearch,
    catalog: &NonSteamSessionCatalog,
    events: &mut Vec<SessionEvent>,
) {
    let results: Vec<SessionInfo> = catalog
        .sessions
        .iter()
        .filter(|(_, entry)| {
            if search.require_open_slot && entry.members.len() >= entry.config.max_members as usize
            {
                return false;
            }
            for (k, v) in &search.metadata {
                match entry.config.metadata.get(k) {
                    Some(val) if val == v => {}
                    _ => return false,
                }
            }
            true
        })
        .take(search.max_results as usize)
        .map(|(id, entry)| entry.to_info(*id))
        .collect();

    events.push(SessionEvent::SearchResults(results));
}

pub(crate) fn handle_join_by_code(
    code: SessionCode,
    identity: &PlayerIdentity,
    nonce: &[u8; 32],
    catalog: &mut NonSteamSessionCatalog,
    state: &mut AfterglowSessionState,
    events: &mut Vec<SessionEvent>,
) {
    let session_id = catalog
        .sessions
        .iter()
        .find(|(_, entry)| entry.code == code)
        .map(|(id, _)| *id);

    match session_id {
        Some(id) => handle_join(id, code.as_str(), identity, nonce, catalog, state, events),
        None => {
            events.push(SessionEvent::Error(SessionError::SessionNotFound));
        }
    }
}

pub(crate) fn handle_join(
    session_id: SessionId,
    target: &str,
    identity: &PlayerIdentity,
    nonce: &[u8; 32],
    catalog: &mut NonSteamSessionCatalog,
    state: &mut AfterglowSessionState,
    events: &mut Vec<SessionEvent>,
) {
    if state.current_session.is_some() {
        events.push(SessionEvent::Error(SessionError::AlreadyInSession));
        return;
    }

    let backend = SessionBackend::NonSteam;
    if validate_identity_for_non_steam(identity, backend, target, nonce).is_err() {
        events.push(SessionEvent::Error(SessionError::PermissionDenied));
        return;
    }

    // Rejoin detection: same native public key or same Steam ID gets the same
    // member slot. Done as a read-only lookup first to avoid borrow conflicts.
    let existing_member = match catalog.sessions.get(&session_id) {
        Some(entry) => match identity {
            PlayerIdentity::Native(proof) => {
                let key: [u8; 32] = match proof.public_key.as_slice().try_into() {
                    Ok(k) => k,
                    Err(_) => {
                        events.push(SessionEvent::Error(SessionError::PermissionDenied));
                        return;
                    }
                };
                entry.key_to_member.get(&key).copied()
            }
            PlayerIdentity::Steam { steam_id, .. } => entry
                .member_identities
                .iter()
                .find(|(_, id)| {
                    matches!(id, PlayerIdentity::Steam { steam_id: sid, .. } if sid == steam_id)
                })
                .map(|(mid, _)| *mid),
        },
        None => {
            events.push(SessionEvent::Error(SessionError::SessionNotFound));
            return;
        }
    };

    let (member_id, _already_member) = match existing_member {
        Some(mid) => {
            // Rejoin: restore the local member id so the rest of the flow can
            // correctly attribute events and later leave/cleanup to this player.
            state.local_member_id = mid;
            (mid, true)
        }
        None => {
            let is_full = catalog
                .sessions
                .get(&session_id)
                .map(|e| e.members.len() >= e.config.max_members as usize)
                .unwrap_or(true);
            if is_full {
                events.push(SessionEvent::Error(SessionError::SessionFull));
                return;
            }
            let mid = ensure_local_member_id(catalog, state);
            if let Some(entry) = catalog.sessions.get_mut(&session_id) {
                entry.members.push(mid);
            }
            (mid, false)
        }
    };

    if let Some(entry) = catalog.sessions.get_mut(&session_id) {
        entry.member_identities.insert(member_id, identity.clone());
        insert_native_key(entry, identity, member_id);
    }

    state.current_session = Some(session_id);
    state.current_backend = Some(SessionBackend::NonSteam);
    state.identity = Some(identity.clone());

    let info = catalog
        .sessions
        .get(&session_id)
        .expect("session must exist after join validation")
        .to_info(session_id);
    events.push(SessionEvent::Joined(info.clone()));
    // Always emit MemberJoined so a remote rejoiner re-learns their own
    // member id after a Leave cleared it locally. Status updates and the
    // SessionStatus.members list deduplicate against already-present ids.
    events.push(SessionEvent::MemberJoined {
        session: session_id,
        member: member_id,
    });
}

pub(crate) fn ensure_local_member_id(
    catalog: &mut NonSteamSessionCatalog,
    state: &mut AfterglowSessionState,
) -> SessionMemberId {
    if !state.local_member_id.is_valid() {
        state.local_member_id = catalog.allocate_member_id();
    }
    state.local_member_id
}

pub(crate) fn handle_leave(
    catalog: &mut NonSteamSessionCatalog,
    state: &mut AfterglowSessionState,
    events: &mut Vec<SessionEvent>,
) {
    let session_id = match state.current_session {
        Some(id) => id,
        None => {
            events.push(SessionEvent::Error(SessionError::NotInSession));
            return;
        }
    };

    let entry = match catalog.sessions.get(&session_id) {
        Some(e) => e,
        None => {
            state.current_session = None;
            state.current_backend = None;
            state.identity = None;
            events.push(SessionEvent::Left {
                session: session_id,
                reason: SessionLeaveReason::HostEnded,
            });
            events.push(SessionEvent::SessionEnded(session_id));
            return;
        }
    };

    let is_owner = entry.owner == state.local_member_id;
    let _code = entry.code.clone();

    if is_owner {
        let non_owner_members: Vec<SessionMemberId> = entry
            .members
            .iter()
            .copied()
            .filter(|m| *m != state.local_member_id)
            .collect();

        for m in &non_owner_members {
            events.push(SessionEvent::MemberLeft {
                session: session_id,
                member: *m,
                reason: SessionLeaveReason::HostEnded,
            });
        }

        // Do not free the session code on leave: codes are derived from a
        // monotonic counter and freed codes could be re-allocated to a later
        // session, enabling replay of an old identity proof signed against
        // the same target string. The u64 counter space is large enough that
        // we never need to recycle.
        // catalog.used_codes.remove(&code);
        catalog.sessions.remove(&session_id);
        state.current_session = None;
        state.current_backend = None;
        state.identity = None;
        state.local_member_id = SessionMemberId::INVALID;
        events.push(SessionEvent::Left {
            session: session_id,
            reason: SessionLeaveReason::Left,
        });
        events.push(SessionEvent::SessionEnded(session_id));
    } else {
        let member_id = state.local_member_id;
        if let Some(entry) = catalog.sessions.get_mut(&session_id) {
            entry.members.retain(|m| *m != member_id);
            if let Some(identity) = entry.member_identities.remove(&member_id) {
                remove_native_key(entry, &identity);
            }
        }
        state.current_session = None;
        state.current_backend = None;
        state.identity = None;
        state.local_member_id = SessionMemberId::INVALID;
        events.push(SessionEvent::Left {
            session: session_id,
            reason: SessionLeaveReason::Left,
        });
        events.push(SessionEvent::MemberLeft {
            session: session_id,
            member: member_id,
            reason: SessionLeaveReason::Left,
        });
    }
}
