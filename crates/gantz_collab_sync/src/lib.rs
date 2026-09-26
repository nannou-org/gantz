//! The host-agnostic sync plane for gantz collaborative sessions.
//!
//! [`gantz_collab`] moves objects between peers and owns nothing
//! node-shaped. This crate is the bookkeeping between that runtime and a
//! local [`gantz_ca::Registry`]. It has no host types, so a windowed app and
//! a headless process drive the same code.
//!
//! Outbound: any local commit marks [`Sessions`] dirty. [`announce`] then
//! mirrors each session's scoped closure into the served
//! [`gantz_collab::SessionRegistry`] and broadcasts the changed tips.
//! Fast-forwards and adoptions of received tips are never re-announced.
//!
//! Inbound: [`poll`] drains the runtime, drives the want and fetch loop
//! through `gantz_ca::sync::Staged` validation, applies completed closures
//! to the registry and converges each scoped name. Names the host has open
//! come back as [`Effect::RemoteTip`], so the host can migrate its live
//! state. Background names move headlessly, followed by a reference resync.
//!
//! Vault: a device links to its owner's vault with [`vault::link`]. Each
//! [`poll`] then plans a step for every name out of step with the vault, and
//! fetches, pushes, merges or moves graphs aside to match. [`serve_push`] is
//! the vault's side of a push.
//!
//! The pure `gantz_ca::sync` rules decide what to merge and in which
//! orientation, and the pure `vault::step` rules decide how each name meets
//! the vault. Everything else here is bookkeeping around them.

pub use inbound::{Effect, OpenHeads, handle_event, poll};
pub use lifecycle::{JoinError, infra, join, leave, resync_headless, session_resolutions, share};
pub use outbound::{announce, serve_scope};
pub use state::{PeerPointer, PendingTip, SessionState, Sessions};
pub use vault::{PushOutcome, VaultLink, serve_push};

mod inbound;
mod lifecycle;
mod outbound;
mod state;
#[cfg(test)]
mod tests;
pub mod vault;
