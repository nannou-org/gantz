//! The wire protocol. Gossip messages and request and response types.
//!
//! Everything here is plain serde encoded with [postcard], which is compact
//! and non-self-describing. `gantz_ca` addresses serialize as raw bytes and
//! names as strings. Graphs and section values are the exception. Erased
//! node data, [`DataGraph`], and section [`Value`]s are self-describing, so
//! they travel inside [`Objects`] as RON blobs. See [`encode_graph`] and
//! [`encode_value`]. That is the same encoding as the persisted registry in
//! `gantz_store`, so wire and persistence cannot drift. A received
//! graph only applies if its decoded content re-verifies against the
//! announced address. See [`gantz_ca::verify_graph`].
//!
//! The human-facing `.gantz` text format is deliberately not used here. It
//! is a name-resolving projection for import and export, and its round-trip
//! re-seeds names and re-roots commits. Sync ships bare address-keyed graphs
//! and moves names only through the convergence rules.
//!
//! [postcard]: https://docs.rs/postcard

use crate::{
    session::{PeerId, SessionId},
    vault::{NameState, PairingSecret, Push},
};
use gantz_ca::{
    BlobLiveness, Commit, CommitAddr, ContentAddr, DataGraph, GraphAddr, Key, Liveness,
    MergePolicy, Name, SectionId, Value,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// A message broadcast on a session's gossip topic.
///
/// Must stay well under iroh-gossip's message-size limit of 4 KiB by default.
/// Anything bulky moves over the request plane instead.
///
/// Variant order is part of the wire format. Postcard discriminants follow
/// declaration order, so append new variants at the end.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum GossipMsg {
    /// Scoped names whose tips changed on the announcing peer, with the tips'
    /// graph addresses. Receivers use those to pre-check twin adoptions
    /// cheaply.
    Tips {
        origin: PeerId,
        /// Per-origin sequence number, for stale-drop only. Convergence
        /// never depends on delivery order.
        seq: u64,
        changed: Vec<(Name, CommitAddr, GraphAddr)>,
    },
    /// An anti-entropy digest of the announcing peer's scoped heads. See
    /// [`heads_digest`].
    ///
    /// Reserved. Nothing broadcasts digests yet, and receivers do not pull
    /// [`SyncRequest::Heads`] on mismatch, though the server already answers
    /// it. A peer that misses a `Tips` broadcast re-heals on the next
    /// announcement. This variant is the wire slot for the planned
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
    /// An ephemeral application-encoded node-interaction action, such as a
    /// live widget gesture or an eval trigger. Fire-and-forget. It is never
    /// persisted and has no convergence obligation, so the commit plane is
    /// unaffected when these drop.
    Action {
        origin: PeerId,
        /// Per-origin sequence number, for stale-drop only.
        seq: u64,
        /// Sender wall-clock milliseconds since the epoch. The cross-origin
        /// last-write-wins tiebreak for value-shaped actions, and history
        /// display.
        timestamp: u64,
        /// The scoped branch name the action's head was on.
        name: Name,
        /// The graph address the action was issued against. Node-index
        /// paths are only meaningful relative to a specific graph.
        /// Receivers apply an action only while their tip holds the
        /// identical graph, and drop it otherwise.
        graph: GraphAddr,
        /// The application-encoded action, opaque here like graph blobs. An
        /// undecodable or unknown action drops alone without poisoning the
        /// envelope.
        data: Vec<u8>,
    },
    /// An ephemeral pointer position over a shared graph, a presence cursor.
    /// Fire-and-forget. Receivers expire stale entries, and the next
    /// movement corrects a lost message.
    Pointer {
        origin: PeerId,
        /// Per-origin sequence number, for stale-drop only. Gossip may
        /// reorder, and a cursor jumping backwards would be visible.
        seq: u64,
        /// The scoped branch name the pointer is over.
        name: Name,
        /// The pointer position in graph-space coordinates. Those are
        /// camera-independent, so every peer renders it correctly regardless
        /// of viewport. `None` means the pointer left the scene.
        pos: Option<(f32, f32)>,
    },
}

/// The size cap for [`GossipMsg::Action`]'s application-encoded `data`.
///
/// Keeps the whole message comfortably inside iroh-gossip's 4 KiB limit.
/// Senders drop an oversized action with a warning rather than truncate it.
pub const MAX_ACTION_DATA: usize = 2048;

/// A kind-tagged reference to one content-addressed object.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ObjectRef {
    Commit(CommitAddr),
    Graph(GraphAddr),
    /// A blob in the named blob section. The wire slot for asset transfer.
    Blob {
        section: SectionId,
        addr: ContentAddr,
    },
    /// A metadata section entry, for example a stored scene view.
    Section {
        id: SectionId,
        key: Key,
    },
    /// Everything reachable from `tips` that is not reachable from `have`,
    /// in one response. See `gantz_ca::closure_diff`. Commits come
    /// newest-first with their graphs, blobs and keyed sections. A large
    /// history is cut at a commit boundary, so the requester asks again for
    /// the frontier it still misses.
    Closure {
        tips: Vec<CommitAddr>,
        have: Vec<CommitAddr>,
    },
    /// A name's metadata: its entry in every section keyed by name. See
    /// [`crate::store::name_meta`].
    Meta(Name),
}

/// A fetched object under its claimed reference.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Object {
    Commit(CommitAddr, WireCommit),
    /// A graph as a RON-serialized [`DataGraph`] blob. See [`encode_graph`].
    Graph(GraphAddr, Vec<u8>),
    /// Raw blob bytes, with the store liveness that stamps the section if
    /// the receiver does not hold it yet.
    Blob {
        section: SectionId,
        liveness: BlobLiveness,
        addr: ContentAddr,
        bytes: Vec<u8>,
    },
    /// A metadata section entry. The section's stamped semantics ride along,
    /// as [`Object::Blob`]'s `liveness` does, so a receiver without the
    /// owning domain compiled in still stamps the section correctly. The
    /// value is a RON blob. See [`encode_value`]. Section values can hold
    /// [`gantz_ca::Datum`]s, which only self-describing formats can decode.
    Section {
        id: SectionId,
        policy: MergePolicy,
        liveness: Liveness,
        key: Key,
        value: Vec<u8>,
    },
}

/// Objects a peer is missing. The wire form of `gantz_ca::sync::Missing`.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Want {
    pub refs: Vec<ObjectRef>,
}

/// Fetched session content.
///
/// Order carries no meaning. Receivers validate and topologically apply via
/// `gantz_ca::sync::Staged`.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Objects {
    pub objects: Vec<Object>,
}

/// [`Commit`] mirrored without serde field-skipping.
///
/// `Commit` omits an empty `merge_parents` from its serialized form for
/// persisted-registry compatibility. That desynchronises non-self-describing
/// readers like postcard, since the reader cannot tell the field is absent.
/// The wire carries this faithful mirror instead.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct WireCommit {
    pub timestamp: gantz_ca::Timestamp,
    pub parent: Option<CommitAddr>,
    pub graph: GraphAddr,
    pub merge_parents: Vec<CommitAddr>,
}

/// A request over the [`crate::SYNC_ALPN`] plane. One request per QUIC
/// bi-stream.
///
/// Variant order is part of the wire format. Append new variants at the end.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum SyncRequest {
    /// Protocol negotiation and access check. A vault pairs an unknown peer
    /// that presents its secret. See [`crate::vault`]. `app` names the
    /// sender's app and version, for display.
    Hello {
        session: SessionId,
        proto: u32,
        pairing: Option<PairingSecret>,
        app: String,
    },
    /// The full served store, for a joiner's initial sync.
    Snapshot { session: SessionId },
    /// The scoped name to tip map, for anti-entropy pulls.
    Heads { session: SessionId },
    /// Specific missing objects.
    Want { session: SessionId, want: Want },
    /// Follow a vault's heads. The stream stays open, carrying
    /// length-prefixed [`WatchMsg`](crate::WatchMsg) frames.
    Watch { session: SessionId },
    /// Move one name on a vault. Answered with [`SyncResponse::Pushed`].
    Push { session: SessionId, push: Push },
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
    /// The pushed name's state on the vault after the push. See
    /// [`crate::PushReply::send`].
    Pushed {
        state: NameState,
    },
}

impl SyncRequest {
    /// The session or vault the request addresses.
    pub fn session(&self) -> SessionId {
        match self {
            Self::Hello { session, .. }
            | Self::Snapshot { session }
            | Self::Heads { session }
            | Self::Want { session, .. }
            | Self::Watch { session }
            | Self::Push { session, .. } => *session,
        }
    }
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
    // Postcard serialization of plain enums and structs cannot fail short of
    // allocation failure.
    postcard::to_allocvec(value).unwrap_or_default()
}

/// Decode a wire value with postcard.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, postcard::Error> {
    postcard::from_bytes(bytes)
}

/// Encode a graph as its wire blob. RON of the erased [`DataGraph`], the
/// same self-describing encoding as the persisted registry.
pub fn encode_graph(graph: &DataGraph) -> Vec<u8> {
    // RON serialization of plain data cannot fail short of allocation
    // failure.
    ron::to_string(graph).unwrap_or_default().into_bytes()
}

/// Decode a graph wire blob. See [`encode_graph`].
///
/// Decoding proves nothing. The caller must verify the decoded graph
/// against the address it was announced under. See
/// [`gantz_ca::verify_graph`].
pub fn decode_graph(bytes: &[u8]) -> Result<DataGraph, ron::de::SpannedError> {
    ron::de::from_bytes(bytes)
}

/// Encode a section value as its wire blob. RON of the [`Value`], the same
/// self-describing encoding as the persisted registry. Self-description is
/// required, since [`Value::Datum`] cannot ride a non-self-describing format
/// like postcard.
pub fn encode_value(value: &Value) -> Vec<u8> {
    // RON serialization of plain data cannot fail short of allocation
    // failure.
    ron::to_string(value).unwrap_or_default().into_bytes()
}

/// Decode a section value wire blob. See [`encode_value`].
///
/// Section entries are advisory metadata with no content address to verify
/// against. Receivers skip entries that fail to decode.
pub fn decode_value(bytes: &[u8]) -> Result<Value, ron::de::SpannedError> {
    ron::de::from_bytes(bytes)
}

/// The digest of a name to tip head map, for [`GossipMsg::Digest`]
/// anti-entropy. It is blake3 over the `(name, tip)` pairs in iteration
/// order.
///
/// Callers must supply a name-ordered iteration, such as
/// `gantz_ca::Registry::heads`, so peers holding equal heads derive equal
/// digests. Only heads are digested. A whole-sections digest would need a
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

    /// One message of every [`GossipMsg`] variant, in declaration order.
    fn gossip_msgs() -> [GossipMsg; 5] {
        let ca = CommitAddr::from(gantz_ca::ContentAddr::from([3; 32]));
        let ga = GraphAddr::from(gantz_ca::ContentAddr::from([4; 32]));
        [
            GossipMsg::Tips {
                origin: PeerId([1; 32]),
                seq: 7,
                changed: vec![(name("main"), ca, ga)],
            },
            GossipMsg::Digest {
                origin: PeerId([2; 32]),
                seq: 5,
                n_names: 1,
                digest: [6; 32],
            },
            GossipMsg::Presence {
                origin: PeerId([1; 32]),
                name: Some("ann".to_string()),
            },
            GossipMsg::Action {
                origin: PeerId([9; 32]),
                seq: 3,
                timestamp: 1_000_000,
                name: name("main"),
                graph: ga,
                data: vec![1, 2, 3],
            },
            GossipMsg::Pointer {
                origin: PeerId([7; 32]),
                seq: 9,
                name: name("main"),
                pos: Some((1.5, -2.0)),
            },
        ]
    }

    /// The wire types have no `PartialEq`. Postcard gives distinct values
    /// distinct bytes, so equal bytes after a round trip show an equal value.
    #[test]
    fn wire_types_round_trip() {
        for msg in gossip_msgs() {
            let bytes = encode(&msg);
            let decoded: GossipMsg = decode(&bytes).unwrap();
            assert_eq!(encode(&decoded), bytes, "{msg:?}");
        }

        let ca = CommitAddr::from(gantz_ca::ContentAddr::from([3; 32]));
        let ga = GraphAddr::from(gantz_ca::ContentAddr::from([4; 32]));
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
        let bytes = encode(&req);
        let decoded: SyncRequest = decode(&bytes).unwrap();
        assert_eq!(encode(&decoded), bytes, "{req:?}");
    }

    /// Variant order is part of the wire format. An appended variant must
    /// not shift the postcard discriminants of the variants before it.
    #[test]
    fn gossip_discriminants_are_pinned() {
        for (tag, msg) in gossip_msgs().iter().enumerate() {
            assert_eq!(usize::from(encode(msg)[0]), tag, "{msg:?}");
        }
    }

    #[test]
    fn objects_round_trip_ordinary_commits() {
        // `Commit`'s skip-when-empty `merge_parents` cannot ride postcard
        // directly. The wire mirror must round-trip an ordinary commit with
        // no merge parents faithfully.
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

    /// A `Value::Datum` cannot decode from a non-self-describing format,
    /// since `Datum` deserializes via `deserialize_any`. So section values
    /// must travel as RON blobs inside the postcard envelope.
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

    /// A graph keeps its address through the wire, which carries graphs as
    /// RON. RON does not record whether an integer was signed.
    #[test]
    fn graphs_verify_after_the_wire() {
        use gantz_ca::{Datum, NodeData};
        let ints = vec![Datum::I64(3), Datum::I64(-3), Datum::U64(u64::MAX)];
        let data = Datum::Map(vec![("n".into(), Datum::Seq(ints))]);
        let mut graph = DataGraph::default();
        graph.add_node(NodeData::new("Test", data));
        let decoded = decode_graph(&encode_graph(&graph)).unwrap();
        gantz_ca::verify_graph(gantz_ca::graph_addr(&graph), &decoded).unwrap();
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

    #[test]
    fn vault_wire_types_round_trip_and_leave_other_variants_stable() {
        let ca = CommitAddr::from(gantz_ca::ContentAddr::from([3; 32]));
        let session = SessionId([2; 32]);
        let closure = ObjectRef::Closure {
            tips: vec![ca],
            have: vec![],
        };
        assert_eq!(decode::<ObjectRef>(&encode(&closure)).unwrap(), closure);
        assert_eq!(encode(&closure)[0], 4);

        let push = SyncRequest::Push {
            session,
            push: Push {
                name: name("jam"),
                tip: Some(ca),
                base: None,
                objects: Objects::default(),
                meta: None,
            },
        };
        let SyncRequest::Push { push: decoded, .. } = decode(&encode(&push)).unwrap() else {
            panic!("wrong variant");
        };
        assert_eq!(
            (decoded.name, decoded.tip, decoded.base),
            (name("jam"), Some(ca), None)
        );
        assert_eq!(encode(&SyncRequest::Watch { session })[0], 4);
        assert_eq!(encode(&push)[0], 5);
        assert_eq!(
            encode(&SyncRequest::Want {
                session,
                want: Want::default()
            })[0],
            3
        );

        let state = NameState {
            head: Some(ca),
            meta: Some(gantz_ca::ContentAddr::from([4; 32])),
        };
        let pushed = SyncResponse::Pushed { state };
        let SyncResponse::Pushed { state: decoded } = decode(&encode(&pushed)).unwrap() else {
            panic!("wrong variant");
        };
        assert_eq!(decoded, state);
        assert_eq!(encode(&pushed)[0], 5);

        let changed = crate::WatchMsg::Changed {
            name: name("jam"),
            state,
        };
        let crate::WatchMsg::Changed { name: n, state: s } = decode(&encode(&changed)).unwrap()
        else {
            panic!("wrong variant");
        };
        assert_eq!((n, s), (name("jam"), state));
        let meta = ObjectRef::Meta(name("jam"));
        assert_eq!(decode::<ObjectRef>(&encode(&meta)).unwrap(), meta);
    }
}
