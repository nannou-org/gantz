//! The wire protocol: gossip messages and request/response types.
//!
//! Everything here is plain serde encoded with [postcard] (compact,
//! non-self-describing; `gantz_ca` addresses serialize as raw bytes and
//! names as strings). Graphs and section values are the exception: erased
//! node data ([`DataGraph`]) and section [`Value`]s are self-describing, so
//! they travel inside [`Objects`] as RON blobs
//! ([`encode_graph`]/[`decode_graph`], [`encode_value`]/[`decode_value`]) -
//! the same encoding as the persisted registry (`bevy_gantz::storage`), so
//! wire and persistence cannot drift. A received graph only applies if its
//! decoded content re-verifies against the announced address (see
//! [`gantz_ca::verify_graph`]). The human-facing `.gantz` text format is
//! deliberately not used here: it is a name-resolving projection for
//! import/export (its round-trip re-seeds names and re-roots commits),
//! while sync ships bare address-keyed graphs and moves names only through
//! the convergence rules.
//!
//! [postcard]: https://docs.rs/postcard

use crate::session::{PeerId, SessionId};
use gantz_ca::{
    BlobLiveness, Commit, CommitAddr, ContentAddr, DataGraph, GraphAddr, Key, Liveness,
    MergePolicy, Name, SectionId, Value,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// A message broadcast on a session's gossip topic.
///
/// Must stay well under iroh-gossip's message-size limit (4 KiB by default):
/// anything bulky moves over the request plane instead.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum GossipMsg {
    /// Scoped names whose tips changed on the announcing peer, with the tips'
    /// graph addresses (so receivers can pre-check twin adoptions cheaply).
    Tips {
        origin: PeerId,
        /// Per-origin sequence number, for stale-drop only: convergence
        /// never depends on delivery order.
        seq: u64,
        changed: Vec<(Name, CommitAddr, GraphAddr)>,
    },
    /// Anti-entropy: a digest of the announcing peer's scoped heads (see
    /// [`heads_digest`]).
    ///
    /// Reserved: nothing broadcasts digests yet, and receivers do not pull
    /// [`SyncRequest::Heads`] on mismatch (the server already answers it).
    /// Today a peer that misses a `Tips` broadcast re-heals on the next
    /// announcement; this variant is the wire slot for the planned
    /// digest-triggered pull.
    Digest {
        origin: PeerId,
        seq: u64,
        n_names: u32,
        digest: [u8; 32],
    },
    /// Presence and self-reported username.
    Presence {
        origin: PeerId,
        name: Option<String>,
    },
    /// An ephemeral node-interaction action (a live widget gesture or an
    /// eval trigger), application-encoded. Fire-and-forget: never persisted,
    /// no convergence obligation - the commit plane is unaffected when these
    /// drop. Appended after the existing variants so their postcard
    /// discriminants are unchanged.
    Action {
        origin: PeerId,
        /// Per-origin sequence number, for stale-drop only.
        seq: u64,
        /// Sender wall-clock milliseconds since the epoch: the cross-origin
        /// last-write-wins tiebreak for value-shaped actions, and history
        /// display.
        timestamp: u64,
        /// The scoped name (branch) the action's head was on.
        name: Name,
        /// The graph address the action was issued against. Node-index
        /// paths are only meaningful relative to a specific graph:
        /// receivers apply an action only while their tip holds the
        /// identical graph, and drop it otherwise.
        graph: GraphAddr,
        /// The application-encoded action (opaque here, like graph blobs -
        /// an undecodable or unknown action drops alone without poisoning
        /// the envelope).
        data: Vec<u8>,
    },
}

/// The size cap for [`GossipMsg::Action`]'s application-encoded `data`.
///
/// Keeps the whole message comfortably inside iroh-gossip's 4 KiB limit;
/// senders drop (with a warning) rather than truncate an oversized action.
pub const MAX_ACTION_DATA: usize = 2048;

/// A kind-tagged reference to one content-addressed object.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ObjectRef {
    Commit(CommitAddr),
    Graph(GraphAddr),
    /// A blob in the named blob section (the wire slot for asset transfer).
    Blob {
        section: SectionId,
        addr: ContentAddr,
    },
    /// A metadata section entry (e.g. a stored scene view).
    Section {
        id: SectionId,
        key: Key,
    },
}

/// A fetched object under its claimed reference.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Object {
    Commit(CommitAddr, WireCommit),
    /// A graph as a RON-serialized [`DataGraph`] blob (see [`encode_graph`]).
    Graph(GraphAddr, Vec<u8>),
    /// Raw blob bytes, with the store liveness that stamps the section if
    /// the receiver does not hold it yet.
    Blob {
        section: SectionId,
        liveness: BlobLiveness,
        addr: ContentAddr,
        bytes: Vec<u8>,
    },
    /// A metadata section entry. The section's stamped semantics ride along
    /// (the same pattern as [`Object::Blob`]'s `liveness`) so a receiver
    /// without the owning domain compiled in still stamps the section
    /// correctly. The value is a RON blob (see [`encode_value`]): section
    /// values can hold [`gantz_ca::Datum`]s, which only self-describing
    /// formats can decode.
    Section {
        id: SectionId,
        policy: MergePolicy,
        liveness: Liveness,
        key: Key,
        value: Vec<u8>,
    },
}

/// Objects a peer is missing (the wire form of `gantz_ca::sync::Missing`).
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Want {
    pub refs: Vec<ObjectRef>,
}

/// Fetched session content.
///
/// Order carries no meaning: receivers validate and topologically apply via
/// `gantz_ca::sync::Staged`.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Objects {
    pub objects: Vec<Object>,
}

/// [`Commit`] mirrored without serde field-skipping.
///
/// `Commit` deliberately omits an empty `merge_parents` from its serialized
/// form (persisted-registry compatibility), which desynchronises
/// non-self-describing readers like postcard - the reader cannot tell the
/// field is absent. The wire carries this faithful mirror instead.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct WireCommit {
    pub timestamp: gantz_ca::Timestamp,
    pub parent: Option<CommitAddr>,
    pub graph: GraphAddr,
    pub merge_parents: Vec<CommitAddr>,
}

/// A request over the [`SYNC_ALPN`](crate::SYNC_ALPN) plane; one request per
/// QUIC bi-stream.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum SyncRequest {
    /// Protocol negotiation and access check.
    Hello { session: SessionId, proto: u32 },
    /// The full served store: a joiner's initial sync.
    Snapshot { session: SessionId },
    /// The scoped `name -> tip` map, for anti-entropy pulls.
    Heads { session: SessionId },
    /// Specific missing objects.
    Want { session: SessionId, want: Want },
}

/// The response to a [`SyncRequest`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum SyncResponse {
    Hello {
        proto: u32,
        accepted: bool,
    },
    Snapshot {
        heads: Vec<(Name, CommitAddr)>,
        objects: Objects,
    },
    Heads {
        heads: Vec<(Name, CommitAddr)>,
    },
    Objects(Objects),
    /// Unknown session, failed access check or protocol mismatch.
    Denied {
        reason: String,
    },
}

impl Want {
    /// Whether nothing is wanted.
    pub fn is_empty(&self) -> bool {
        self.refs.is_empty()
    }
}

impl From<Commit> for WireCommit {
    fn from(c: Commit) -> Self {
        Self {
            timestamp: c.timestamp,
            parent: c.parent,
            graph: c.graph,
            merge_parents: c.merge_parents,
        }
    }
}

impl From<WireCommit> for Commit {
    fn from(c: WireCommit) -> Self {
        Self {
            timestamp: c.timestamp,
            parent: c.parent,
            graph: c.graph,
            merge_parents: c.merge_parents,
        }
    }
}

/// Encode a wire value with postcard.
pub fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    // Postcard serialization of our plain enums/structs cannot fail short of
    // allocation failure.
    postcard::to_allocvec(value).unwrap_or_default()
}

/// Decode a wire value with postcard.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, postcard::Error> {
    postcard::from_bytes(bytes)
}

/// Encode a graph as its wire blob: RON of the erased [`DataGraph`], the
/// same self-describing encoding as the persisted registry.
pub fn encode_graph(graph: &DataGraph) -> Vec<u8> {
    // RON serialization of plain data cannot fail short of allocation
    // failure.
    ron::to_string(graph).unwrap_or_default().into_bytes()
}

/// Decode a graph wire blob (see [`encode_graph`]).
///
/// Decoding proves nothing: the caller must verify the decoded graph
/// against the address it was announced under (see
/// [`gantz_ca::verify_graph`]).
pub fn decode_graph(bytes: &[u8]) -> Result<DataGraph, ron::de::SpannedError> {
    ron::de::from_bytes(bytes)
}

/// Encode a section value as its wire blob: RON of the [`Value`], the same
/// self-describing encoding as the persisted registry. Self-description is
/// required: [`Value::Datum`] cannot ride a non-self-describing format like
/// postcard.
pub fn encode_value(value: &Value) -> Vec<u8> {
    // RON serialization of plain data cannot fail short of allocation
    // failure.
    ron::to_string(value).unwrap_or_default().into_bytes()
}

/// Decode a section value wire blob (see [`encode_value`]).
///
/// Section entries are advisory metadata with no content address to verify
/// against: receivers skip entries that fail to decode.
pub fn decode_value(bytes: &[u8]) -> Result<Value, ron::de::SpannedError> {
    ron::de::from_bytes(bytes)
}

/// The digest of a `name -> tip` head map, for [`GossipMsg::Digest`]
/// anti-entropy: blake3 over the `(name, tip)` pairs in iteration order.
///
/// Callers must supply a name-ordered iteration (e.g.
/// `gantz_ca::Registry::heads`) so peers holding equal heads derive equal
/// digests. Heads-only for now: a whole-sections digest would need a
/// canonical section byte encoding, which the registry does not define yet.
pub fn heads_digest<'a>(heads: impl IntoIterator<Item = (&'a Name, CommitAddr)>) -> [u8; 32] {
    let mut hasher = gantz_ca::Hasher::new();
    for (name, tip) in heads {
        let name = name.to_string();
        hasher.update(&(name.len() as u64).to_be_bytes());
        hasher.update(name.as_bytes());
        hasher.update(&gantz_ca::ContentAddr::from(tip).0);
    }
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn name(s: &str) -> Name {
        s.parse().unwrap()
    }

    #[test]
    fn wire_types_round_trip() {
        let ca = CommitAddr::from(gantz_ca::ContentAddr::from([3; 32]));
        let ga = GraphAddr::from(gantz_ca::ContentAddr::from([4; 32]));
        let msg = GossipMsg::Tips {
            origin: PeerId([1; 32]),
            seq: 7,
            changed: vec![(name("main"), ca, ga)],
        };
        let decoded: GossipMsg = decode(&encode(&msg)).unwrap();
        let GossipMsg::Tips {
            origin,
            seq,
            changed,
        } = decoded
        else {
            panic!("wrong variant");
        };
        assert_eq!(origin, PeerId([1; 32]));
        assert_eq!(seq, 7);
        assert_eq!(changed, vec![(name("main"), ca, ga)]);

        let req = SyncRequest::Want {
            session: SessionId([2; 32]),
            want: Want {
                refs: vec![
                    ObjectRef::Commit(ca),
                    ObjectRef::Graph(ga),
                    ObjectRef::Blob {
                        section: "dsp.buffer".to_string(),
                        addr: gantz_ca::ContentAddr::from([5; 32]),
                    },
                    ObjectRef::Section {
                        id: "egui.view".to_string(),
                        key: Key::Commit(ca),
                    },
                ],
            },
        };
        let decoded: SyncRequest = decode(&encode(&req)).unwrap();
        let SyncRequest::Want { want, .. } = decoded else {
            panic!("wrong variant");
        };
        assert_eq!(want.refs.len(), 4);
    }

    #[test]
    fn action_round_trips_and_leaves_other_variants_stable() {
        let ga = GraphAddr::from(gantz_ca::ContentAddr::from([4; 32]));
        let msg = GossipMsg::Action {
            origin: PeerId([9; 32]),
            seq: 3,
            timestamp: 1_000_000,
            name: name("main"),
            graph: ga,
            data: vec![1, 2, 3],
        };
        let decoded: GossipMsg = decode(&encode(&msg)).unwrap();
        let GossipMsg::Action {
            origin,
            seq,
            timestamp,
            name,
            graph,
            data,
        } = decoded
        else {
            panic!("wrong variant");
        };
        assert_eq!(origin, PeerId([9; 32]));
        assert_eq!(seq, 3);
        assert_eq!(timestamp, 1_000_000);
        assert_eq!(name, "main".parse::<Name>().unwrap());
        assert_eq!(graph, ga);
        assert_eq!(data, vec![1, 2, 3]);

        // The new trailing variant must not shift the existing postcard
        // discriminants: a pre-action Tips encoding still starts with tag 0.
        let tips = GossipMsg::Tips {
            origin: PeerId([1; 32]),
            seq: 0,
            changed: vec![],
        };
        assert_eq!(encode(&tips)[0], 0);
        let presence = GossipMsg::Presence {
            origin: PeerId([1; 32]),
            name: None,
        };
        assert_eq!(encode(&presence)[0], 2);
    }

    #[test]
    fn objects_round_trip_ordinary_commits() {
        // Regression: `Commit`'s skip-when-empty `merge_parents` cannot ride
        // postcard directly - the wire mirror must round-trip an ordinary
        // (merge-parent-free) commit faithfully.
        let ga = GraphAddr::from(gantz_ca::ContentAddr::from([4; 32]));
        let commit = Commit::new(std::time::Duration::from_secs(5), None, ga);
        let ca = gantz_ca::commit_addr(&commit);
        let objects = Objects {
            objects: vec![
                Object::Commit(ca, commit.clone().into()),
                Object::Graph(ga, b"blob".to_vec()),
                Object::Blob {
                    section: "dsp.buffer".to_string(),
                    liveness: BlobLiveness::ContentReferenced,
                    addr: gantz_ca::blob_addr(b"pcm"),
                    bytes: b"pcm".to_vec(),
                },
                Object::Section {
                    id: "egui.view".to_string(),
                    policy: MergePolicy::KeepExisting,
                    liveness: Liveness::WithCommit,
                    key: Key::Commit(ca),
                    value: encode_value(&Value::Datum(gantz_ca::Datum::Bool(true))),
                },
            ],
        };
        let decoded: Objects = decode(&encode(&objects)).unwrap();
        assert_eq!(decoded, objects);
    }

    /// Regression: a `Value::Datum` cannot decode from a non-self-describing
    /// format (`Datum` deserializes via `deserialize_any`), so section
    /// values must travel as RON blobs inside the postcard envelope.
    #[test]
    fn section_values_round_trip_as_ron_not_postcard() {
        let value = Value::Datum(gantz_ca::Datum::Map(vec![(
            "zoom".to_string(),
            gantz_ca::Datum::F64(1.5),
        )]));
        assert_eq!(decode_value(&encode_value(&value)).unwrap(), value);
        let postcard_err: Result<Value, _> = decode(&encode(&value));
        assert!(postcard_err.is_err());
    }

    #[test]
    fn heads_digest_is_order_independent_and_content_sensitive() {
        let ca = |n| CommitAddr::from(gantz_ca::ContentAddr::from([n; 32]));
        let digest = |entries: &[(&str, CommitAddr)]| {
            // A `BTreeMap` supplies the required name-ordered iteration
            // regardless of insertion order.
            let map: BTreeMap<Name, CommitAddr> =
                entries.iter().map(|(n, ca)| (name(n), *ca)).collect();
            heads_digest(map.iter().map(|(n, ca)| (n, *ca)))
        };
        let a = digest(&[("a", ca(1)), ("b", ca(2))]);
        let b = digest(&[("b", ca(2)), ("a", ca(1))]);
        assert_eq!(a, b);
        let c = digest(&[("a", ca(9))]);
        assert_ne!(a, c);
    }
}
