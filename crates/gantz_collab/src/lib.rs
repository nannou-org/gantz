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
pub use session::{Access, ConnState, PeerId, Role, Session, SessionId};
#[doc(inline)]
pub use store::{SessionEntry, SessionRegistry};
#[doc(inline)]
pub use ticket::SessionTicket;

pub mod identity;
pub mod proto;
pub mod runtime;
pub mod session;
pub mod store;
pub mod ticket;
