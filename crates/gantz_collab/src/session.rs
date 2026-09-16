//! Session identity and configuration types.
//!
//! These are plain serde types. Peers are identified by their ed25519 public
//! key bytes as [`PeerId`] and sessions by 32 random bytes as [`SessionId`].
//! Conversion to iroh's types is confined to the [`crate::runtime`].

use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fmt};

/// A peer's identity. Its ed25519 public key bytes, iroh's `EndpointId`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
pub struct PeerId(pub [u8; 32]);

/// A session's unique identifier. 32 random bytes, minted by the sharing
/// peer. Seeds the session's gossip topic and appears in tickets.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
pub struct SessionId(pub [u8; 32]);

/// Who may read and contribute to a session.
///
/// Access is enforced on the request plane, which is the data plane. Every
/// connection is authenticated by iroh, and restricted sessions answer only
/// allowlisted peers. Gossip metadata such as names, tip addresses and
/// presence is visible to anyone holding the ticket.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub enum Access {
    /// Anyone with the ticket may join.
    #[default]
    Public,
    /// Only the listed peers may join.
    Restricted(BTreeSet<PeerId>),
}

/// This peer's role in a session.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum Role {
    /// This peer created the session and enforces its access control.
    Host,
    /// This peer joined via a ticket.
    Guest,
}

/// The persisted sharing configuration for one shared graph.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Session {
    /// The session's unique identifier.
    pub id: SessionId,
    /// The shared branch name this session syncs.
    pub branch: String,
    /// Who may join.
    pub access: Access,
    /// The fixed conflict-resolution policy every peer applies, so that
    /// independently performed merges converge.
    pub resolutions: gantz_ca::merge::Resolutions,
    /// This peer's role.
    pub role: Role,
}

/// A session's connection lifecycle, for the GUI indicator.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ConnState {
    /// Subscribed, awaiting the first peer or the join snapshot.
    #[default]
    Connecting,
    /// At least one peer is reachable and the initial sync completed.
    Live,
    /// No peers reachable. Local edits continue and re-heal on reconnect.
    Degraded,
}

impl SessionId {
    /// A fresh random session id.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        // Failure to source OS randomness is unrecoverable and cannot mint a
        // usable session. Surface it loudly rather than share under a
        // predictable id.
        getrandom::fill(&mut bytes).expect("failed to source randomness for a session id");
        Self(bytes)
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        display_short(&self.0, f)
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        display_short(&self.0, f)
    }
}

/// The first four bytes as lowercase hex, enough to eyeball identity.
fn display_short(bytes: &[u8; 32], f: &mut fmt::Formatter<'_>) -> fmt::Result {
    for b in &bytes[..4] {
        write!(f, "{b:02x}")?;
    }
    Ok(())
}
