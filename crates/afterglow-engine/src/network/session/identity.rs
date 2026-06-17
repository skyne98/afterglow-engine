use crate::network::session::SessionBackend;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Durable player identity used to authenticate join requests.
///
/// `SessionMemberId` is only a per-session handle. `PlayerIdentity` is the
/// anti-spoofing/persistence boundary. It is intentionally modeled like
/// [`SessionBackend`] so that Steam, Xbox, PlayStation, or an Afterglow-native
/// keypair can plug in behind the same API.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PlayerIdentity {
    /// Afterglow-native identity: the client holds an Ed25519 private key and
    /// proves possession by signing a server nonce.
    Native(NativeIdentityProof),
    /// Steam identity. The `ticket` is passed through to Steam validation in
    /// the Steam backend. The non-Steam provider accepts the shape but does
    /// not validate Steam tickets.
    Steam { steam_id: u64, ticket: Vec<u8> },
}

impl PlayerIdentity {
    /// Build a native proof for demos using a deterministic key.
    ///
    /// `key_seed` produces distinct Ed25519 keys so demos can simulate multiple
    /// players. The caller provides the server `nonce` and the `target` string
    /// (e.g. `"create"` for session creation, or a session code for joins).
    pub fn demo(nonce: &[u8; 32], target: &str, key_seed: u8) -> Self {
        use ed25519_dalek::{Signer, SigningKey};
        let mut secret = [0u8; 32];
        for i in 0..32 {
            secret[i] = key_seed.wrapping_add(i as u8).wrapping_mul(7);
        }
        let signing_key = SigningKey::from_bytes(&secret);
        let challenge = NativeIdentityProof::challenge(SessionBackend::NonSteam, target, nonce);
        let signature = signing_key.sign(&challenge);
        Self::Native(NativeIdentityProof {
            public_key: signing_key.verifying_key().to_bytes().to_vec(),
            signature: signature.to_bytes().to_vec(),
        })
    }

    /// Build a native proof for tests by signing the canonical challenge with a
    /// deterministic key.
    ///
    /// `key_seed` produces distinct Ed25519 keys so tests can simulate multiple
    /// players. Seed `0` is the default test key.
    #[cfg(test)]
    pub fn test_native(
        nonce: &[u8; 32],
        backend: SessionBackend,
        target: &str,
        key_seed: u8,
    ) -> Self {
        Self::Native(NativeIdentityProof::test(nonce, backend, target, key_seed))
    }
}

/// Native Afterglow identity proof.
///
/// The client signs the canonical challenge message produced by
/// [`NativeIdentityProof::challenge`] with an Ed25519 private key it generated
/// and stores locally. The server verifies the signature with the included
/// public key, proving possession without ever seeing the private key.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeIdentityProof {
    pub public_key: Vec<u8>,
    pub signature: Vec<u8>,
}

impl NativeIdentityProof {
    /// Build the canonical byte message that a native identity proof must sign.
    ///
    /// `target` depends on the request:
    /// - `Create`: `"create"`
    /// - `Join { session }`: the decimal string of the session id
    /// - `JoinByCode { code }`: the session code string
    pub fn challenge(backend: SessionBackend, target: &str, nonce: &[u8; 32]) -> Vec<u8> {
        let mut msg = Vec::with_capacity(64 + target.len());
        msg.extend_from_slice(b"afterglow-session:");
        msg.push(backend as u8);
        msg.extend_from_slice(target.as_bytes());
        msg.extend_from_slice(nonce);
        msg
    }

    /// Verify that this proof's signature matches the challenge.
    pub fn verify(
        &self,
        backend: SessionBackend,
        target: &str,
        nonce: &[u8; 32],
    ) -> Result<(), IdentityError> {
        use ed25519_dalek::{Signature, VerifyingKey};

        let public_key: [u8; 32] = self
            .public_key
            .as_slice()
            .try_into()
            .map_err(|_| IdentityError::InvalidPublicKey)?;
        let signature_bytes: [u8; 64] = self
            .signature
            .as_slice()
            .try_into()
            .map_err(|_| IdentityError::InvalidSignature)?;

        let verifying_key =
            VerifyingKey::from_bytes(&public_key).map_err(|_| IdentityError::InvalidPublicKey)?;
        let signature = Signature::from_bytes(&signature_bytes);
        let message = Self::challenge(backend, target, nonce);

        verifying_key
            .verify_strict(&message, &signature)
            .map_err(|_| IdentityError::InvalidSignature)
    }

    #[cfg(test)]
    fn test(nonce: &[u8; 32], backend: SessionBackend, target: &str, key_seed: u8) -> Self {
        use ed25519_dalek::{Signer, SigningKey};

        // Deterministic test key derived from `key_seed`. We avoid uniform
        // byte patterns like all-zeros or all-ones because they can produce
        // weak public keys that `verify_strict` rejects.
        let mut secret = [0u8; 32];
        for i in 0..32 {
            secret[i] = key_seed.wrapping_add(i as u8).wrapping_mul(7);
        }
        let signing_key = SigningKey::from_bytes(&secret);
        let message = Self::challenge(backend, target, nonce);
        let signature = signing_key.sign(&message);

        Self {
            public_key: signing_key.verifying_key().to_bytes().to_vec(),
            signature: signature.to_bytes().to_vec(),
        }
    }
}

/// Errors that can occur while validating a player identity proof.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityError {
    InvalidPublicKey,
    InvalidSignature,
    UnsupportedBackend,
}

/// Server-side nonce used when verifying native identity proofs.
///
/// A new nonce is generated from the OS CSPRNG when the resource is created.
/// In tests the resource can be replaced with a fixed nonce so signatures are
/// deterministic.
#[derive(Resource, Clone, Debug)]
pub struct SessionIdentityNonce(pub [u8; 32]);

impl Default for SessionIdentityNonce {
    fn default() -> Self {
        use rand_core::RngCore;
        let mut bytes = [0u8; 32];
        rand_core::OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }
}
