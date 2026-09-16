//! The user's collaborative identity: an ed25519 key pair.
//!
//! Generated implicitly on first use and persisted by the application. The
//! secret is 32 bytes. Deriving the key from another source later is a new
//! constructor, not a type change.

use crate::session::PeerId;
use iroh::SecretKey;

/// An ed25519 key pair identifying this user across sessions.
///
/// The public half is the [`PeerId`] other peers see and allowlist. The
/// secret half authenticates every connection via iroh.
#[derive(Clone, Debug)]
pub struct Identity {
    secret: SecretKey,
}

impl Identity {
    /// Generate a fresh random identity.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        // Failure to source OS randomness cannot produce a usable secret
        // identity. Surface it loudly rather than continue predictably.
        getrandom::fill(&mut bytes).expect("failed to source randomness for an identity");
        Self::from_bytes(bytes)
    }

    /// Reconstruct an identity from its persisted secret bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self {
            secret: SecretKey::from_bytes(&bytes),
        }
    }

    /// The secret bytes, for persistence.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.secret.to_bytes()
    }

    /// The public identity other peers see.
    pub fn peer_id(&self) -> PeerId {
        PeerId(*self.secret.public().as_bytes())
    }

    /// The underlying iroh secret key.
    pub(crate) fn secret_key(&self) -> SecretKey {
        self.secret.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_round_trips_through_bytes() {
        let id = Identity::generate();
        let restored = Identity::from_bytes(id.to_bytes());
        assert_eq!(id.peer_id(), restored.peer_id());
    }

    #[test]
    fn generated_identities_are_distinct() {
        assert_ne!(
            Identity::generate().peer_id(),
            Identity::generate().peer_id()
        );
    }
}
