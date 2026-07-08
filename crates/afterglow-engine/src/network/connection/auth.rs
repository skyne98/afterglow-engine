//! Identity storage, challenge-response authentication messages, and the
//! server-side `On<Add, ClientOf>` observer that drives the auth flow.

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::{
    ConnectionConfig, ConnectionEvent, ConnectionEventKind, MemberLinkMap, PlayerId,
    readiness::add_channels_to_transport,
};

use ed25519_dalek::{Signer, Verifier};

// ---------------------------------------------------------------------------
// Auth messages
// ---------------------------------------------------------------------------

/// Server-to-client challenge: "prove you own the private key for your
/// claimed client_id by signing this nonce."
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ChallengeMessage {
    pub nonce: [u8; 32],
}

/// Client-to-server response: the public key and the signature over the
/// nonce the server sent.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AuthResponse {
    pub public_key: [u8; 32],
    /// Ed25519 signature (64 bytes).
    pub signature: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Server-side pending-auth state
// ---------------------------------------------------------------------------

/// Attached to a `ClientOf` link entity while we wait for the client to
/// return a valid `AuthResponse`.
#[derive(Component, Clone, Debug)]
pub(crate) struct PendingAuth {
    pub player_id: PlayerId,
    pub nonce: [u8; 32],
}

// ---------------------------------------------------------------------------
// LocalIdentity (NonSteam)
// ---------------------------------------------------------------------------

/// A player's persistent identity.
///
/// For NonSteam: an Ed25519 keypair stored at `~/.afterglow/identity.key`.
/// For Steam: the `SteamId` and no keypair.
///
/// The engine loads/creates this identity at app startup and exposes it as
/// a resource.
#[derive(Resource, Clone, Debug)]
pub struct LocalIdentity {
    /// The stable player identifier (= netcode client_id).
    pub player_id: PlayerId,
    /// The Ed25519 signing key, if available (NonSteam).
    /// `None` for Steam (identity is proven by Steam backend).
    pub keypair: Option<ed25519_dalek::SigningKey>,
}

impl LocalIdentity {
    /// Load an existing keypair from disk or generate a new one.
    ///
    /// The keypair is persisted to `~/.afterglow/identity.key` (32-byte seed).
    /// The `player_id` is derived as `blake3(public_key)[0..8]`.
    pub fn load_or_create() -> Self {
        Self::load_or_create_named("")
    }

    /// Load or create an identity, scoping the keypair file to the player name.
    /// This prevents two players on the same machine from sharing a keypair
    /// (and thus the same `client_id`).
    pub fn load_or_create_named(name: &str) -> Self {
        let path = identity_file_path(name);
        let keypair = (|| -> Option<ed25519_dalek::SigningKey> {
            let bytes = std::fs::read(&path).ok()?;
            let arr: [u8; 32] = bytes[..].try_into().ok()?;
            Some(ed25519_dalek::SigningKey::from_bytes(&arr))
        })()
        .unwrap_or_else(|| generate_keypair(&path));

        let verifying_key = keypair.verifying_key();
        let public_key = verifying_key.to_bytes();
        let player_id = hash_public_key(&public_key);

        Self {
            player_id,
            keypair: Some(keypair),
        }
    }

    /// Create an identity without a keypair (for tests or when auth is
    /// handled externally, e.g. Steam).
    pub fn unauthenticated(player_id: PlayerId) -> Self {
        Self {
            player_id,
            keypair: None,
        }
    }

    /// The Ed25519 public key bytes, if available.
    pub fn public_key(&self) -> Option<[u8; 32]> {
        self.keypair.as_ref().map(|k| k.verifying_key().to_bytes())
    }
}

fn identity_file_path(name: &str) -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    let mut path = PathBuf::from(home);
    path.push(".afterglow");
    if name.is_empty() {
        path.push("identity.key");
    } else {
        path.push(format!("identity-{}.key", name));
    }
    path
}

fn generate_keypair(path: &PathBuf) -> ed25519_dalek::SigningKey {
    use rand_core::OsRng;

    let signing_key = ed25519_dalek::SigningKey::generate(&mut OsRng);
    if let Some(parent) = path.parent() {
        // Silently ignore failures (read-only FS, etc.).
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, signing_key.to_bytes());
    // Set restrictive permissions on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    signing_key
}

/// Hash an Ed25519 public key to a `u64` player id.
pub fn hash_public_key(pubkey: &[u8; 32]) -> PlayerId {
    let hash = blake3::hash(pubkey);
    u64::from_le_bytes(hash.as_bytes()[..8].try_into().unwrap())
}

// ---------------------------------------------------------------------------
// Server observer: On<Add, ClientOf>
// ---------------------------------------------------------------------------

/// Synchronous observer that fires when the Lightyear netcode server plugin
/// adds `ClientOf` to a new link entity (netcode handshake complete).
///
/// Responsibilities:
/// 1. Ensure `ReplicationSender` exists on the link.
/// 2. Add transport channels (Metadata, Updates, Actions, Input).
/// 3. Populate [`MemberLinkMap`].
/// 4. If `require_auth`: send `ChallengeMessage`, store `PendingAuth`. Else:
///    emit `ConnectionEvent::Connected`.
#[allow(clippy::too_many_arguments)]
pub fn on_client_of_added(
    trigger: On<Add, lightyear::prelude::server::ClientOf>,
    mut commands: Commands,
    connection_config: Option<Res<ConnectionConfig>>,
    registry: Option<Res<ChannelRegistry>>,
    mut member_links: ResMut<MemberLinkMap>,
    // Transport must be mutable to add channels.
    mut q_transport: Query<&mut Transport>,
    // Read-only link info.
    q_info: Query<(&RemoteId, Option<&ReplicationSender>)>,
    // The entity should already have MessageSender<ChallengeMessage> and
    // MessageReceiver<AuthResponse> via register_required_components.
    mut q_challenge_sender: Query<&mut MessageSender<ChallengeMessage>>,
) {
    let entity = trigger.entity;

    let Ok((remote_id, sender)) = q_info.get(entity) else {
        return;
    };
    let needs_sender = sender.is_none();

    let PeerId::Netcode(client_id) = remote_id.0 else {
        // Not a netcode peer; skip (local / crossbeam).
        return;
    };
    let player_id = client_id as PlayerId;

    // The per-client link needs a Transport with all registered channels
    // before Lightyear's replication sender systems run. If required
    // components already supplied ReplicationSender/Transport, mutate the
    // transport in-place. Otherwise repair the missing components.
    use std::time::Duration;
    let Some(ref reg) = registry else {
        bevy::log::error!(
            "ChannelRegistry is missing while configuring client {}; check plugin ordering",
            player_id
        );
        return;
    };

    let name = Name::from(format!("RemoteClient-{}", player_id));
    let mut link_commands = commands.entity(entity);
    if let Ok(mut transport) = q_transport.get_mut(entity) {
        add_channels_to_transport(&mut transport, reg);
        if needs_sender {
            link_commands.insert((
                ReplicationSender::new(
                    Duration::from_millis(16),
                    SendUpdatesMode::SinceLastAck,
                    false,
                ),
                name,
            ));
        } else {
            link_commands.insert(name);
        }
    } else {
        let mut transport = Transport::default();
        add_channels_to_transport(&mut transport, reg);
        if needs_sender {
            link_commands.insert((
                ReplicationSender::new(
                    Duration::from_millis(16),
                    SendUpdatesMode::SinceLastAck,
                    false,
                ),
                name,
                transport,
            ));
        } else {
            link_commands.insert((name, transport));
        }
    }

    // 2. Auth or direct connect. Authenticated links enter MemberLinkMap only
    // after proof succeeds; unauthenticated links must not become control owners.
    let config = connection_config.as_deref().cloned().unwrap_or_default();

    if config.require_auth {
        // Send challenge. If the sender component is unavailable, fail closed:
        // a pending auth link with no challenge can never complete safely.
        let nonce = generate_nonce();
        if let Ok(mut sender) = q_challenge_sender.get_mut(entity) {
            sender.send::<ActionsChannel>(ChallengeMessage { nonce });
            commands
                .entity(entity)
                .insert(PendingAuth { player_id, nonce });
        } else {
            bevy::log::error!(
                "ChallengeMessage sender missing on client link {}; disconnecting",
                entity
            );
            commands.entity(entity).insert(Disconnected {
                reason: Some("auth challenge sender missing".into()),
            });
        }
    } else {
        member_links.links.insert(player_id, entity);
        commands.trigger(ConnectionEvent {
            kind: ConnectionEventKind::Connected,
            player_id,
            link_entity: entity,
        });
    }
}

/// Synchronous observer that turns a normal Lightyear disconnect on a
/// server-side `ClientOf` link into the engine-level lifecycle event that games
/// use to despawn player entities.
pub fn on_client_disconnected(
    trigger: On<Add, Disconnected>,
    mut commands: Commands,
    mut member_links: ResMut<MemberLinkMap>,
    q_link: Query<(&RemoteId, &Disconnected), With<lightyear::prelude::server::ClientOf>>,
) {
    let entity = trigger.entity;
    let Ok((remote_id, disconnected)) = q_link.get(entity) else {
        return;
    };
    let PeerId::Netcode(client_id) = remote_id.0 else {
        return;
    };
    let player_id = client_id as PlayerId;
    member_links.links.remove(&player_id);
    commands.trigger(ConnectionEvent {
        kind: ConnectionEventKind::Disconnected {
            reason: disconnected
                .reason
                .clone()
                .unwrap_or_else(|| "disconnected".into()),
        },
        player_id,
        link_entity: entity,
    });
}

// ---------------------------------------------------------------------------
// Auth-response verification (server side)
// ---------------------------------------------------------------------------

/// System that reads `AuthResponse` messages from client links and verifies
/// the challenge-response.
pub(crate) fn receive_auth_response(
    mut commands: Commands,
    connection_config: Option<Res<ConnectionConfig>>,
    mut member_links: ResMut<MemberLinkMap>,
    mut q_receiver: Query<(Entity, &mut MessageReceiver<AuthResponse>, &PendingAuth)>,
) {
    let config = connection_config.as_deref().cloned().unwrap_or_default();
    if !config.require_auth {
        return;
    }

    for (entity, mut receiver, pending) in &mut q_receiver {
        // Drain the receiver but process at most one response. PendingAuth removal
        // is deferred, so duplicate responses in one packet batch must not emit
        // duplicate ConnectionEvent::Connected events.
        let Some(msg) = receiver.receive().next() else {
            continue;
        };
        let AuthResponse {
            public_key,
            signature,
        } = msg;

        // Verify: blake3(pubkey)[..8] == claimed client_id.
        let derived_id = hash_public_key(&public_key);
        if derived_id != pending.player_id {
            bevy::log::warn!(
                "AuthResponse client_id mismatch: claimed {}, derived {}",
                pending.player_id,
                derived_id,
            );
            // Disconnect the client and clear pending auth state.
            commands
                .entity(entity)
                .insert(lightyear::prelude::Disconnected {
                    reason: Some("Auth failed: client_id mismatch".into()),
                })
                .remove::<PendingAuth>();
            member_links.links.remove(&pending.player_id);
            continue;
        }

        // Verify signature over nonce.
        let Ok(verifying_key) = ed25519_dalek::VerifyingKey::from_bytes(&public_key) else {
            bevy::log::warn!("AuthResponse invalid public key");
            commands
                .entity(entity)
                .insert(lightyear::prelude::Disconnected {
                    reason: Some("Auth failed: invalid public key".into()),
                })
                .remove::<PendingAuth>();
            member_links.links.remove(&pending.player_id);
            continue;
        };

        let Ok(sig) = ed25519_dalek::Signature::from_slice(&signature) else {
            bevy::log::warn!("AuthResponse invalid signature bytes");
            commands
                .entity(entity)
                .insert(lightyear::prelude::Disconnected {
                    reason: Some("Auth failed: invalid signature".into()),
                })
                .remove::<PendingAuth>();
            member_links.links.remove(&pending.player_id);
            continue;
        };

        match verifying_key.verify(&pending.nonce, &sig) {
            Ok(()) => {
                // Success! Emit Connected.
                bevy::log::info!("Client {} authenticated successfully", pending.player_id,);
                member_links.links.insert(pending.player_id, entity);
                commands.entity(entity).remove::<PendingAuth>();
                commands.trigger(ConnectionEvent {
                    kind: ConnectionEventKind::Connected,
                    player_id: pending.player_id,
                    link_entity: entity,
                });
            }
            Err(e) => {
                bevy::log::warn!(
                    "Auth signature verification failed for {}: {:?}",
                    pending.player_id,
                    e,
                );
                commands
                    .entity(entity)
                    .insert(lightyear::prelude::Disconnected {
                        reason: Some(format!("Auth failed: signature verification error: {e:?}")),
                    })
                    .remove::<PendingAuth>();
                member_links.links.remove(&pending.player_id);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Challenge receiver (client side)
// ---------------------------------------------------------------------------

/// System that reads `ChallengeMessage` from the server and responds with
/// `AuthResponse` signed by the local identity.
pub(crate) fn receive_challenge(
    mut commands: Commands,
    connection_config: Option<Res<ConnectionConfig>>,
    identity: Option<Res<LocalIdentity>>,
    mut q_receiver: Query<(
        Entity,
        &mut MessageReceiver<ChallengeMessage>,
        Option<&mut MessageSender<AuthResponse>>,
    )>,
) {
    let config = connection_config.as_deref().cloned().unwrap_or_default();
    if !config.require_auth {
        return;
    }

    for (entity, mut receiver, sender) in &mut q_receiver {
        // Drain the receiver and handle at most one challenge per link. Auth is
        // a one-shot handshake; duplicate challenges in the same packet batch
        // should not generate duplicate AuthResponse messages.
        let Some(msg) = receiver.receive().next() else {
            continue;
        };

        let Some(identity) = identity.as_deref() else {
            bevy::log::warn!("Received auth challenge without LocalIdentity; disconnecting");
            commands.entity(entity).insert(Disconnected {
                reason: Some("auth challenge received without LocalIdentity".into()),
            });
            continue;
        };
        let Some(ref keypair) = identity.keypair else {
            bevy::log::warn!(
                "Received auth challenge for unauthenticated LocalIdentity {}; disconnecting",
                identity.player_id
            );
            commands.entity(entity).insert(Disconnected {
                reason: Some("auth challenge requires signing identity".into()),
            });
            continue;
        };
        let Some(mut sender) = sender else {
            bevy::log::warn!("Received auth challenge but AuthResponse sender is missing");
            commands.entity(entity).insert(Disconnected {
                reason: Some("auth response sender missing".into()),
            });
            continue;
        };

        let signature = keypair.sign(&msg.nonce);
        sender.send::<ActionsChannel>(AuthResponse {
            public_key: keypair.verifying_key().to_bytes(),
            signature: signature.to_bytes().to_vec(),
        });
        bevy::log::info!("Sent auth response for player {}", identity.player_id,);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Generate a random 32-byte nonce for the challenge.
fn generate_nonce() -> [u8; 32] {
    use rand_core::RngCore;
    let mut nonce = [0u8; 32];
    rand_core::OsRng.fill_bytes(&mut nonce);
    nonce
}
