//! Peer-to-peer collaborative session networking for gantz.
//!
//! A session shares one named graph and its registry dependency closure
//! between peers over [iroh]. This crate owns everything network-shaped and
//! nothing node-shaped. Graphs sit in the served [`SessionRegistry`] in their
//! erased data form, [`gantz_ca::DataGraph`], and cross the wire as RON blobs
//! via [`proto::encode_graph`]. So the crate stays agnostic of the
//! application's node types while it re-hashes and verifies every graph it
//! serves. Merging and applying received content is the application layer's
//! job, built on `gantz_ca::sync`.
//!
//! There are two planes:
//!
//! - Gossip, via [`GossipMsg`] on a per-session topic. Tip announcements,
//!   anti-entropy digests and presence. Small, broadcast, unordered.
//! - Requests, via [`SYNC_ALPN`] with one request per QUIC bi-stream. Join
//!   snapshots, head listings and object fetches as [`SyncRequest`] and
//!   [`SyncResponse`]. Served from the session's [`SessionRegistry`] and
//!   gated by its [`Access`] allowlist.
//!
//! The [`runtime`] drives an iroh endpoint on a dedicated thread on native or
//! the browser's event loop on wasm. Plain [`Command`] and [`Event`] channels
//! bridge it to the application. The channels are unbounded and the runtime
//! owns the served stores, so the application never blocks on a lock. Store
//! content rides ordered [`Command::Register`] and [`Command::Update`] sends.
//!
//! A [`vault`] syncs all of a user's named graphs between their devices. It
//! rides the request plane alone. Devices link to one vault peer, which
//! referees every change to a name.
//!
//! ## Versions
//!
//! Peers decide whether they can sync on integer versions alone. App
//! version strings are for display, since development builds share one.
//!
//! - The sync protocol, [`PROTO_VERSION`]. It is part of [`SYNC_ALPN`]. A
//!   [`version`] probe on the frozen [`VERSION_ALPN`] tells a device which
//!   versions its vault speaks before it says hello. Bump the protocol for
//!   a breaking wire change, for a message that a server sends without a
//!   request, such as a new [`WatchMsg`] variant, and for a change to how
//!   content addresses are computed.
//! - The store format, `gantz_store::STORE_FORMAT`. A build does not
//!   write a store in a newer format. Bump it when a stored entry changes
//!   shape or meaning. An address change bumps both.
//! - The vault ticket layout. A build reports a ticket with a newer layout.
//!
//! A newer vault should keep serving the previous protocol version for one
//! release. Users can then update the vault first and each device at its
//! own pace. A device that finds its vault on another protocol pauses sync
//! and tells which side must update. Nothing is lost while it waits.
//!
//! Node data is not versioned. When a node field is retired, keep it for
//! one release as an optional field that is skipped when absent, so that
//! older builds still read it.
//!
//! [iroh]: https://docs.rs/iroh

#[doc(inline)]
pub use identity::Identity;
#[doc(inline)]
pub use proto::{
    GossipMsg, Object, ObjectRef, Objects, SyncRequest, SyncResponse, Want, WireCommit,
    heads_digest,
};
#[doc(inline)]
pub use runtime::{
    Command, Event, Handle, Infra, PROTO_VERSION, RuntimeConfig, SYNC_ALPN, TOPIC_DOMAIN, spawn,
};
#[doc(inline)]
pub use session::{Access, ConnState, ParsePeerIdError, PeerId, Role, Session, SessionId};
#[doc(inline)]
pub use store::{SessionEntry, SessionRegistry};
#[doc(inline)]
pub use ticket::{SessionTicket, VaultTicket};
#[doc(inline)]
pub use vault::{PairingSecret, Push, PushReply, VaultEntry, VaultId, WatchMsg};
#[doc(inline)]
pub use version::{Outdated, PROTO_MAX, PROTO_MIN, VERSION_ALPN, VersionInfo};

pub mod identity;
pub mod proto;
pub mod runtime;
pub mod session;
pub mod store;
pub mod ticket;
pub mod vault;
pub mod version;
