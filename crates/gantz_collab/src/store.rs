//! The served session content, owned by the network runtime.
//!
//! Each session serves a plain [`gantz_ca::Registry`]: graphs sit at rest in
//! their erased data form ([`gantz_ca::DataGraph`]), which any peer can
//! re-hash and walk without the application's node types compiled in. The
//! application mirrors each session's scoped closure (commits, graphs,
//! heads) into the store via [`Command::Register`](crate::Command::Register)
//! and [`Command::Update`](crate::Command::Update); the runtime's request
//! handler answers peers from it synchronously, serializing graphs to the
//! wire with [`proto::encode_graph`]. Content-addressed keys make every
//! insert idempotent, so updates may be re-sent freely.
//!
//! The store VERIFIES the graphs it accepts: every graph offered to
//! [`merge`] is re-hashed against its claimed address and checked for node
//! canonicality (see [`gantz_ca::verify_graph`]), so tampered or aliased
//! content is rejected at the store boundary rather than trusted under a
//! claimed address. Receiving peers still re-verify everything through the
//! [`gantz_ca::sync::Staged`] path on their own side. Holding decodable data
//! graphs also means a serving peer can answer reachability questions
//! itself (see [`gantz_ca::closure`]) - e.g. for future served-store GC -
//! which the old opaque-bytes relay store could not.

use crate::{
    proto::{self, Object, ObjectRef, Objects, Want},
    session::{Access, PeerId, Session, SessionId},
};
use gantz_ca::{
    BlobLiveness, Bytes, Commit, CommitAddr, ContentAddr, DataGraph, GraphAddr, HEADS_ID, Key,
    Liveness, MergePolicy, MergeReport, Name, Registry, Section, SectionId, Value, blob_addr,
    sync::VerifyError, verify_graph,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

/// One session's served content. See the module docs.
pub type SessionRegistry = Registry;

/// A session's configuration plus its served content.
#[derive(Debug)]
pub struct SessionEntry {
    pub session: Session,
    pub store: SessionRegistry,
}

/// The state shared between the runtime's driver and its request-serving
/// tasks. Internal: the application mutates it only through the ordered,
/// non-blocking command channel, so it never takes (or waits on) this lock.
#[derive(Debug, Default)]
pub(crate) struct SharedState {
    pub sessions: HashMap<SessionId, SessionEntry>,
}

/// A cheaply clonable handle to the [`SharedState`].
///
/// Lock hold times must stay short (lookups, inserts and response clones
/// only), and every holder runs on the runtime's own thread (native) or the
/// single browser thread (wasm) - contention never involves the
/// application's frame loop.
#[derive(Clone, Debug, Default)]
pub(crate) struct Shared(Arc<Mutex<SharedState>>);

impl SessionEntry {
    /// Whether `peer` may read this session's content.
    pub fn allows(&self, peer: PeerId) -> bool {
        match &self.session.access {
            Access::Public => true,
            Access::Restricted(allowed) => allowed.contains(&peer),
        }
    }
}

impl Shared {
    /// Lock the shared state. A poisoned lock (a panicked peer thread) still
    /// yields the data: content-addressed state cannot be half-written into
    /// an invalid shape.
    pub(crate) fn lock(&self) -> MutexGuard<'_, SharedState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Merge served content into the store: content-addressed commit/graph/blob
/// inserts (idempotent), per-name head upserts (an incoming tip wins,
/// reported) and section entries applied per the section's merge policy
/// (see [`gantz_ca::Registry::merge`]).
///
/// Every graph and blob is verified against its claimed address before
/// anything is merged (graphs: strict re-hash plus node canonicality, see
/// [`gantz_ca::verify_graph`]; blobs: [`gantz_ca::blob_addr`] re-hash): an
/// `Err` leaves the store untouched, so tampered or aliased content can
/// never be served. Section entries are advisory metadata with no address
/// to verify.
pub fn merge(
    store: &mut SessionRegistry,
    heads: impl IntoIterator<Item = (Name, CommitAddr)>,
    commits: impl IntoIterator<Item = (CommitAddr, Commit)>,
    graphs: impl IntoIterator<Item = (GraphAddr, DataGraph)>,
    sections: impl IntoIterator<Item = (SectionId, MergePolicy, Liveness, Key, Value)>,
    blobs: impl IntoIterator<Item = (SectionId, BlobLiveness, ContentAddr, Bytes)>,
) -> Result<MergeReport, VerifyError> {
    let graphs: HashMap<GraphAddr, DataGraph> = graphs.into_iter().collect();
    for (ga, graph) in &graphs {
        verify_graph(*ga, graph)?;
    }
    let blobs: Vec<_> = blobs.into_iter().collect();
    for (_, _, claimed, bytes) in &blobs {
        let actual = blob_addr(bytes);
        if actual != *claimed {
            return Err(VerifyError::Blob {
                claimed: *claimed,
                actual,
            });
        }
    }
    let commits = commits.into_iter().collect();
    let heads = heads.into_iter().collect();
    let mut incoming = Registry::from_parts(graphs, commits, heads);
    for (section, liveness, _, bytes) in blobs {
        incoming.add_blob(section, liveness, bytes);
    }
    for (id, policy, liveness, key, value) in sections {
        incoming.set_section_value(id, policy, liveness, key, value);
    }
    Ok(store.merge(incoming))
}

/// The requested objects, where present. Absent objects are skipped: the
/// requester re-requests from another peer or re-heals on the next announce.
///
/// Every answered commit carries its commit-keyed section entries along
/// (e.g. the commit's stored scene view): a requester cannot know which
/// entries exist, so they piggyback on the commit they describe.
pub fn objects(store: &SessionRegistry, want: &Want) -> Objects {
    let mut objects = Vec::new();
    for r in &want.refs {
        match r {
            ObjectRef::Commit(ca) => {
                let Some(c) = store.commits().get(ca) else {
                    continue;
                };
                objects.push(Object::Commit(*ca, c.clone().into()));
                objects.extend(commit_sections(store, *ca));
            }
            ObjectRef::Graph(ga) => {
                if let Some(g) = store.graph(ga) {
                    objects.push(Object::Graph(*ga, proto::encode_graph(g)));
                }
            }
            ObjectRef::Blob { section, addr } => {
                if let Some(blobs) = store.blobs().get(section) {
                    if let Some(bytes) = blobs.get(addr) {
                        objects.push(Object::Blob {
                            section: section.clone(),
                            liveness: blobs.liveness,
                            addr: *addr,
                            bytes: bytes.to_vec(),
                        });
                    }
                }
            }
            ObjectRef::Section { id, key } => {
                if let Some(section) = store.section(id) {
                    if let Some(value) = section.entries.get(key) {
                        objects.push(section_object(id, section, key, value));
                    }
                }
            }
        }
    }
    Objects { objects }
}

/// The whole store as a join snapshot: every head, commit, graph, blob and
/// non-head section entry (heads travel in the dedicated head list).
pub fn snapshot(store: &SessionRegistry) -> (Vec<(Name, CommitAddr)>, Objects) {
    let heads = store.heads().map(|(n, ca)| (n.clone(), ca)).collect();
    let mut objects = Vec::new();
    for (ca, c) in store.commits() {
        objects.push(Object::Commit(*ca, c.clone().into()));
    }
    for (ga, g) in store.graphs() {
        objects.push(Object::Graph(*ga, proto::encode_graph(g)));
    }
    for (section, blobs) in store.blobs() {
        for (addr, bytes) in &blobs.entries {
            objects.push(Object::Blob {
                section: section.clone(),
                liveness: blobs.liveness,
                addr: *addr,
                bytes: bytes.to_vec(),
            });
        }
    }
    for (id, section) in store.sections() {
        if id.as_str() == HEADS_ID {
            continue;
        }
        for (key, value) in &section.entries {
            objects.push(section_object(id, section, key, value));
        }
    }
    (heads, Objects { objects })
}

/// The entry as its wire object, with the section's stamped semantics.
fn section_object(id: &SectionId, section: &Section, key: &Key, value: &Value) -> Object {
    Object::Section {
        id: id.clone(),
        policy: section.policy,
        liveness: section.liveness,
        key: key.clone(),
        value: proto::encode_value(value),
    }
}

/// Every non-head section entry keyed by the given commit.
fn commit_sections(store: &SessionRegistry, ca: CommitAddr) -> impl Iterator<Item = Object> + '_ {
    let key = Key::Commit(ca);
    store
        .sections()
        .iter()
        .filter(|(id, _)| id.as_str() != HEADS_ID)
        .filter_map(move |(id, section)| {
            let value = section.entries.get(&key)?;
            Some(section_object(id, section, &key, value))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_ca::{BlobLiveness, ContentAddr, Datum, GraphAddr, NodeData, commit_addr};
    use std::time::Duration;

    fn name(s: &str) -> Name {
        s.parse().unwrap()
    }

    /// A one-node graph tagged `tag`, with its content address.
    fn graph(tag: &str) -> (GraphAddr, DataGraph) {
        let mut g = DataGraph::default();
        g.add_node(NodeData::new(tag, Datum::Map(vec![])));
        (gantz_ca::graph_addr(&g), g)
    }

    /// A store with one head over a two-commit chain and its graphs.
    fn test_store() -> (SessionRegistry, CommitAddr, CommitAddr, GraphAddr) {
        let mut store = SessionRegistry::default();
        let (ga1, g1) = graph("g1");
        let (ga2, g2) = graph("g2");
        let root = Commit::new(Duration::from_secs(1), None, ga1);
        let root_ca = commit_addr(&root);
        let tip = Commit::new(Duration::from_secs(2), Some(root_ca), ga2);
        let tip_ca = commit_addr(&tip);
        merge(
            &mut store,
            [(name("jam"), tip_ca)],
            [(root_ca, root), (tip_ca, tip)],
            [(ga1, g1), (ga2, g2)],
            [],
            [],
        )
        .unwrap();
        (store, root_ca, tip_ca, ga1)
    }

    #[test]
    fn merge_is_idempotent() {
        let (mut store, root_ca, tip_ca, ga1) = test_store();
        let root = store.commits()[&root_ca].clone();
        let tip = store.commits()[&tip_ca].clone();
        let g1 = store.graph(&ga1).unwrap().clone();
        let report = merge(
            &mut store,
            [(name("jam"), tip_ca)],
            [(root_ca, root), (tip_ca, tip)],
            [(ga1, g1)],
            [],
            [],
        )
        .unwrap();
        assert!(report.heads_added.is_empty());
        assert!(report.heads_replaced.is_empty());
        assert_eq!(store.commits().len(), 2);
        assert_eq!(store.graphs().len(), 2);
    }

    #[test]
    fn merge_repoints_heads() {
        let (mut store, root_ca, tip_ca, _ga1) = test_store();
        let report = merge(&mut store, [(name("jam"), root_ca)], [], [], [], []).unwrap();
        assert_eq!(report.heads_replaced, vec![(name("jam"), tip_ca, root_ca)]);
        assert_eq!(store.head(&name("jam")), Some(root_ca));
    }

    /// The trust-model flip: a graph offered under a claimed address whose
    /// content does not re-hash to it is rejected outright, and the store is
    /// left untouched. The old opaque-bytes relay store could not re-verify
    /// and served whatever it was handed.
    #[test]
    fn merge_rejects_tampered_graph_content() {
        let (mut store, _root_ca, _tip_ca, ga1) = test_store();
        let heads_before = store
            .heads()
            .map(|(n, ca)| (n.clone(), ca))
            .collect::<Vec<_>>();
        // Honest content for `ga1`, then tampered: an extra node the claimed
        // address does not cover.
        let mut tampered = store.graph(&ga1).unwrap().clone();
        tampered.add_node(NodeData::new("evil", Datum::Map(vec![])));
        let actual = gantz_ca::graph_addr(&tampered);
        let commit = Commit::new(Duration::from_secs(3), None, ga1);
        let commit_ca = commit_addr(&commit);
        let err = merge(
            &mut store,
            [(name("jam"), commit_ca)],
            [(commit_ca, commit)],
            [(ga1, tampered)],
            [],
            [],
        )
        .unwrap_err();
        assert_eq!(
            err,
            VerifyError::Graph {
                claimed: ga1,
                actual,
            }
        );
        // Nothing was merged: no new commit, head unmoved.
        assert!(!store.commits().contains_key(&commit_ca));
        assert_eq!(
            store
                .heads()
                .map(|(n, ca)| (n.clone(), ca))
                .collect::<Vec<_>>(),
            heads_before
        );
    }

    /// A graph whose nodes are not in canonical form aliases the same
    /// logical content under a second address: rejected likewise.
    #[test]
    fn merge_rejects_non_canonical_graph() {
        let (mut store, _root_ca, _tip_ca, _ga1) = test_store();
        let mut g = DataGraph::default();
        g.add_node(NodeData::new(
            "test",
            Datum::Map(vec![
                ("b".to_string(), Datum::Null),
                ("a".to_string(), Datum::Bool(true)),
            ]),
        ));
        // The non-canonical form hashes consistently with itself: the
        // canonicality check, not the hash, must reject it.
        let claimed = gantz_ca::graph_addr(&g);
        let err = merge(&mut store, [], [], [(claimed, g)], [], []).unwrap_err();
        assert_eq!(
            err,
            VerifyError::NonCanonicalNode {
                graph: claimed,
                node_ix: 0,
            }
        );
        assert!(store.graph(&claimed).is_none());
    }

    #[test]
    fn objects_filters_to_present() {
        let (store, root_ca, _tip_ca, ga1) = test_store();
        let absent_ca = CommitAddr::from(ContentAddr::from([9; 32]));
        let absent_ga = GraphAddr::from(ContentAddr::from([9; 32]));
        let want = Want {
            refs: vec![
                ObjectRef::Commit(root_ca),
                ObjectRef::Commit(absent_ca),
                ObjectRef::Graph(ga1),
                ObjectRef::Graph(absent_ga),
                ObjectRef::Blob {
                    section: "dsp.buffer".to_string(),
                    addr: ContentAddr::from([9; 32]),
                },
            ],
        };
        let objects = objects(&store, &want).objects;
        assert_eq!(objects.len(), 2);
        assert!(matches!(objects[0], Object::Commit(ca, _) if ca == root_ca));
        let expected = proto::encode_graph(store.graph(&ga1).unwrap());
        assert!(
            matches!(&objects[1], Object::Graph(ga, bytes) if *ga == ga1 && *bytes == expected)
        );
    }

    #[test]
    fn objects_serves_blobs() {
        let (mut store, _root_ca, _tip_ca, _ga1) = test_store();
        let addr = store.add_blob("dsp.buffer", BlobLiveness::ContentReferenced, &b"pcm"[..]);
        let want = Want {
            refs: vec![ObjectRef::Blob {
                section: "dsp.buffer".to_string(),
                addr,
            }],
        };
        let objects = objects(&store, &want).objects;
        assert_eq!(
            objects,
            vec![Object::Blob {
                section: "dsp.buffer".to_string(),
                liveness: BlobLiveness::ContentReferenced,
                addr,
                bytes: b"pcm".to_vec(),
            }]
        );
    }

    /// An `egui.view`-shaped section entry keyed by the given commit.
    fn view_entry(ca: CommitAddr) -> (SectionId, MergePolicy, Liveness, Key, Value) {
        (
            "egui.view".to_string(),
            MergePolicy::KeepExisting,
            Liveness::WithCommit,
            Key::Commit(ca),
            Value::Datum(Datum::Map(vec![("zoom".to_string(), Datum::F64(1.5))])),
        )
    }

    #[test]
    fn merge_applies_sections_per_policy() {
        let (mut store, _root_ca, tip_ca, _ga1) = test_store();
        let (id, policy, liveness, key, value) = view_entry(tip_ca);
        merge(
            &mut store,
            [],
            [],
            [],
            [(id.clone(), policy, liveness, key.clone(), value.clone())],
            [],
        )
        .unwrap();
        assert_eq!(store.section_entry(&id, &key), Some(&value));
        let section = store.section(&id).unwrap();
        assert_eq!(section.policy, policy);
        assert_eq!(section.liveness, liveness);
        // KeepExisting: a differing incoming entry does not clobber.
        let differing = Value::Datum(Datum::Bool(false));
        merge(
            &mut store,
            [],
            [],
            [],
            [(id.clone(), policy, liveness, key.clone(), differing)],
            [],
        )
        .unwrap();
        assert_eq!(store.section_entry(&id, &key), Some(&value));
    }

    #[test]
    fn merge_applies_verified_blobs() {
        let (mut store, _root_ca, _tip_ca, _ga1) = test_store();
        let bytes = Bytes::from(&b"pcm"[..]);
        let addr = blob_addr(&bytes);
        merge(
            &mut store,
            [],
            [],
            [],
            [],
            [(
                "dsp.buffer".to_string(),
                BlobLiveness::ContentReferenced,
                addr,
                bytes.clone(),
            )],
        )
        .unwrap();
        assert_eq!(store.blob("dsp.buffer", &addr), Some(&bytes));
        let store_liveness = store.blobs()["dsp.buffer"].liveness;
        assert_eq!(store_liveness, BlobLiveness::ContentReferenced);
    }

    #[test]
    fn merge_rejects_tampered_blob() {
        let (mut store, _root_ca, _tip_ca, _ga1) = test_store();
        let claimed = blob_addr(b"pcm");
        let err = merge(
            &mut store,
            [],
            [],
            [],
            [],
            [(
                "dsp.buffer".to_string(),
                BlobLiveness::ContentReferenced,
                claimed,
                Bytes::from(&b"tampered"[..]),
            )],
        )
        .unwrap_err();
        assert_eq!(
            err,
            VerifyError::Blob {
                claimed,
                actual: blob_addr(b"tampered"),
            }
        );
        assert!(store.blobs().get("dsp.buffer").is_none());
    }

    #[test]
    fn objects_serves_sections() {
        let (mut store, _root_ca, tip_ca, _ga1) = test_store();
        let (id, policy, liveness, key, value) = view_entry(tip_ca);
        store.set_section_value(id.clone(), policy, liveness, key.clone(), value.clone());
        let absent = Key::Commit(CommitAddr::from(ContentAddr::from([9; 32])));
        let want = Want {
            refs: vec![
                ObjectRef::Section {
                    id: id.clone(),
                    key: key.clone(),
                },
                ObjectRef::Section {
                    id: id.clone(),
                    key: absent,
                },
            ],
        };
        let objects = objects(&store, &want).objects;
        assert_eq!(
            objects,
            vec![Object::Section {
                id,
                policy,
                liveness,
                key,
                value: proto::encode_value(&value),
            }]
        );
    }

    /// A wanted commit carries its commit-keyed section entries along: the
    /// requester cannot know which entries exist.
    #[test]
    fn objects_piggybacks_commit_sections() {
        let (mut store, root_ca, tip_ca, _ga1) = test_store();
        let (id, policy, liveness, key, value) = view_entry(tip_ca);
        store.set_section_value(id.clone(), policy, liveness, key.clone(), value.clone());
        let want = Want {
            refs: vec![ObjectRef::Commit(tip_ca), ObjectRef::Commit(root_ca)],
        };
        let objects = objects(&store, &want).objects;
        // The tip commit, its piggybacked view, then the entry-less root.
        assert_eq!(objects.len(), 3);
        assert!(matches!(objects[0], Object::Commit(ca, _) if ca == tip_ca));
        assert_eq!(
            objects[1],
            Object::Section {
                id,
                policy,
                liveness,
                key,
                value: proto::encode_value(&value),
            }
        );
        assert!(matches!(objects[2], Object::Commit(ca, _) if ca == root_ca));
    }

    #[test]
    fn snapshot_round_trips() {
        let (mut store, _root_ca, tip_ca, _ga1) = test_store();
        store.add_blob("dsp.buffer", BlobLiveness::ContentReferenced, &b"pcm"[..]);
        let (id, policy, liveness, key, value) = view_entry(tip_ca);
        store.set_section_value(id, policy, liveness, key, value);
        let (heads, objects) = snapshot(&store);
        assert_eq!(heads, vec![(name("jam"), tip_ca)]);
        // The `heads` section travels as the dedicated head list, never as
        // section objects.
        assert!(
            !objects.objects.iter().any(
                |o| matches!(o, Object::Section { id, .. } if id.as_str() == gantz_ca::HEADS_ID)
            )
        );
        // Rebuild a store from the snapshot's wire objects, decoding and
        // re-verifying the graph and section bytes as a receiving peer would.
        let mut rebuilt = SessionRegistry::default();
        let mut commits = Vec::new();
        let mut graphs = Vec::new();
        let mut sections = Vec::new();
        for object in objects.objects {
            match object {
                Object::Commit(ca, c) => commits.push((ca, c.into())),
                Object::Graph(ga, bytes) => {
                    graphs.push((ga, proto::decode_graph(&bytes).unwrap()));
                }
                Object::Blob {
                    section,
                    liveness,
                    bytes,
                    ..
                } => {
                    rebuilt.add_blob(section, liveness, bytes);
                }
                Object::Section {
                    id,
                    policy,
                    liveness,
                    key,
                    value,
                } => {
                    let value = proto::decode_value(&value).unwrap();
                    sections.push((id, policy, liveness, key, value));
                }
            }
        }
        merge(&mut rebuilt, heads, commits, graphs, sections, []).unwrap();
        assert_eq!(rebuilt.commits(), store.commits());
        assert_eq!(rebuilt.blobs(), store.blobs());
        assert_eq!(rebuilt.sections(), store.sections());
        assert_eq!(
            rebuilt
                .graphs()
                .keys()
                .collect::<std::collections::HashSet<_>>(),
            store
                .graphs()
                .keys()
                .collect::<std::collections::HashSet<_>>()
        );
        assert_eq!(
            rebuilt.heads().collect::<Vec<_>>(),
            store.heads().collect::<Vec<_>>()
        );
    }
}
