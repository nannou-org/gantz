//! The vault protocol types.
//!
//! A vault is a referee peer holding a registry that a user's devices sync
//! all their named graphs with. Unlike a session, it is client and server.
//! Devices link to the vault, never to each other.
//!
//! A device's link sends [`SyncRequest::Hello`], then holds a
//! [`SyncRequest::Watch`] stream open. The vault writes its heads first as
//! [`WatchMsg::Heads`], then a [`WatchMsg::Changed`] frame for every change.
//! Devices fetch with the ordinary [`SyncRequest::Want`], where
//! [`ObjectRef::Closure`] takes a whole history in one round trip, and move
//! a name with [`SyncRequest::Push`].
//!
//! The vault accepts a push only while its head for the name is still the
//! push's `base`, so no device overwrites a change it has not seen. The
//! runtime forwards each push to the application as
//! [`Event::PushRequest`], which decides, persists and replies. See
//! `gantz_collab_sync::vault` for the device side.
//!
//! A name also has metadata: its entry in every section keyed by name, such
//! as its description. Metadata lives outside history, so an edit to it
//! mints no commit. The vault reports a digest of each name's metadata next
//! to its head. A push may carry a [`MetaChange`], which the vault accepts
//! only while its digest is still the change's `base`. Devices fetch a
//! name's metadata with [`ObjectRef::Meta`].
//!
//! Access is by pairing. The vault ticket carries a [`PairingSecret`]. A
//! `Hello` that presents it from an unknown peer adds that peer to the
//! vault's allowlist and emits [`Event::DeviceSeen`]. Every other request
//! needs an allowlisted peer. A `Hello` for another protocol version never
//! pairs.
//!
//! [`SyncRequest::Hello`]: crate::SyncRequest::Hello
//! [`SyncRequest::Watch`]: crate::SyncRequest::Watch
//! [`SyncRequest::Want`]: crate::SyncRequest::Want
//! [`SyncRequest::Push`]: crate::SyncRequest::Push
//! [`ObjectRef::Closure`]: crate::ObjectRef::Closure
//! [`ObjectRef::Meta`]: crate::ObjectRef::Meta
//! [`Event::PushRequest`]: crate::Event::PushRequest
//! [`Event::DeviceSeen`]: crate::Event::DeviceSeen

use crate::{
    proto::Objects,
    session::{PeerId, SessionId},
};
use gantz_ca::{CommitAddr, ContentAddr, Name, Registry};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fmt};

/// A vault's unique identifier. 32 random bytes, minted when the vault is
/// first created.
pub type VaultId = SessionId;

/// The secret that pairs a new device with a vault. It rides in the vault
/// ticket, so the ticket is as sensitive as a password.
#[derive(Clone, Copy, Deserialize, Serialize)]
pub struct PairingSecret([u8; 32]);

/// A vault's configuration plus its served content.
#[derive(Debug)]
pub struct VaultEntry {
    pub id: VaultId,
    /// The paired devices.
    pub access: BTreeSet<PeerId>,
    pub pairing: PairingSecret,
    pub store: Registry,
}

/// A request to move one name on the vault.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Push {
    pub name: Name,
    /// The new head. `None` removes the name.
    pub tip: Option<CommitAddr>,
    /// The vault head the device made this change against. The vault
    /// rejects the push unless its head is still this.
    pub base: Option<CommitAddr>,
    /// Everything reachable from `tip` that the vault lacks, given `base`.
    pub objects: Objects,
    /// A change to the name's metadata, if any. A change to the metadata
    /// alone pushes the vault's head as both `tip` and `base`.
    pub meta: Option<MetaChange>,
}

/// A change to a name's metadata. See the module docs.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MetaChange {
    /// The vault's metadata digest the device made this change against.
    /// The vault rejects the push unless its digest is still this.
    pub base: Option<ContentAddr>,
    /// The name's metadata after the change, as section objects keyed by
    /// the name. It replaces the vault's. See [`crate::store::name_meta`].
    pub entries: Objects,
}

/// A name's state on a vault.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct NameState {
    /// The head. `None` when the vault does not hold the name.
    pub head: Option<CommitAddr>,
    /// The digest of the name's metadata. `None` when it has none. See
    /// [`crate::store::meta_addr`].
    pub meta: Option<ContentAddr>,
}

/// A frame on a vault's watch stream.
///
/// Variant order is part of the wire format. Append new variants at the end.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum WatchMsg {
    /// Every head the vault holds, and the metadata digest of every name
    /// with metadata. Always the stream's first frame.
    Heads {
        heads: Vec<(Name, CommitAddr)>,
        metas: Vec<(Name, ContentAddr)>,
    },
    /// A name whose head or metadata changed since the previous frame, with
    /// its state now.
    Changed { name: Name, state: NameState },
}

/// The application's answer to a forwarded push. See
/// [`Event::PushRequest`](crate::Event::PushRequest).
#[derive(Debug)]
pub struct PushReply(pub(crate) async_channel::Sender<Result<NameState, String>>);

impl PairingSecret {
    /// A fresh random secret.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        // A predictable secret would let anyone pair. Surface the failure
        // loudly rather than continue.
        getrandom::fill(&mut bytes).expect("failed to source randomness for a pairing secret");
        Self(bytes)
    }

    /// Whether `other` is this secret. The comparison takes the same time
    /// wherever the bytes differ.
    pub fn matches(&self, other: &Self) -> bool {
        self.0
            .iter()
            .zip(&other.0)
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
    }
}

impl PushReply {
    /// Answer with the name's state on the vault after the push. The push
    /// was accepted exactly when the state holds its `tip`, and the digest
    /// of its metadata if it carried a [`MetaChange`].
    pub fn send(self, state: NameState) {
        // The requesting stream may be gone. The answer then has no reader.
        let _ = self.0.try_send(Ok(state));
    }

    /// Refuse a push that cannot apply, such as one whose objects do not
    /// verify. The device learns the reason rather than retrying.
    pub fn refuse(self, reason: String) {
        let _ = self.0.try_send(Err(reason));
    }
}

impl fmt::Debug for PairingSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PairingSecret(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_match_only_themselves() {
        let a = PairingSecret::generate();
        let b = PairingSecret::generate();
        assert!(a.matches(&a));
        assert!(!a.matches(&b));
    }

    #[test]
    fn debug_redacts_the_secret() {
        let secret = PairingSecret([0xab; 32]);
        assert_eq!(format!("{secret:?}"), "PairingSecret(..)");
    }
}
