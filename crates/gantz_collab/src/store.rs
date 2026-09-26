//! The served session content, owned by the network runtime.
//!
//! Each session serves a plain [`gantz_ca::Registry`]. Graphs sit at rest in
//! their erased data form, [`gantz_ca::DataGraph`], which any peer can re-hash
//! and walk without the application's node types compiled in. The application
//! mirrors each session's scoped closure of commits, graphs and heads into the
//! store via [`crate::Command::Register`] and [`crate::Command::Update`]. The
//! runtime's request handler answers peers from it synchronously. It
//! serializes graphs to the wire with [`proto::encode_graph`].
//! Content-addressed keys make every insert idempotent, so updates may be
//! re-sent freely.
//!
//! The store verifies the graphs it accepts. Every graph offered to [`merge`]
//! is re-hashed against its claimed address and checked for node canonicality.
//! See [`gantz_ca::verify_graph`]. Tampered or aliased content is rejected at
//! the store boundary rather than trusted under a claimed address. Receiving
//! peers still re-verify everything through the [`gantz_ca::sync::Staged`]
//! path on their own side. Holding decodable data graphs also means a serving
//! peer can answer reachability questions itself. See [`gantz_ca::closure`].
//!
//! A hosted vault serves its store the same way. The application mirrors each
//! accepted push into it via [`crate::Command::UpdateVault`]. See
//! [`update_vault`].

use crate::{
    proto::{self, Object, ObjectRef, Objects, Want},
    session::{Access, PeerId, Session, SessionId},
    vault::{VaultEntry, VaultId, WatchMsg},
};
use gantz_ca::{
    BlobLiveness, Bytes, Commit, CommitAddr, ContentAddr, DataGraph, GraphAddr, HEADS_ID, Key,
    Liveness, MergePolicy, MergeReport, Name, Registry, Section, SectionId, Value, blob_addr,
    sync::VerifyError, verify_graph,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

/// One session's served content. See the module docs.
pub type SessionRegistry = Registry;

/// The graph and blob bytes one [`ObjectRef::Closure`] answer carries before
/// it stops at the next commit boundary. Well under the runtime's response
/// limit.
const CLOSURE_BUDGET: usize = 8 * 1024 * 1024;

/// A session's configuration plus its served content.
#[derive(Debug)]
pub struct SessionEntry {
    pub session: Session,
    pub store: SessionRegistry,
}

/// The state shared between the runtime's driver and its request-serving
/// tasks. The application mutates it only through the ordered, non-blocking
/// command channel, so it never takes or waits on this lock.
#[derive(Debug, Default)]
pub(crate) struct SharedState {
    pub sessions: HashMap<SessionId, SessionEntry>,
    pub vaults: HashMap<VaultId, ServedVault>,
}

/// A hosted vault plus the channels of its open watch streams.
#[derive(Debug)]
pub(crate) struct ServedVault {
    pub entry: VaultEntry,
    pub watchers: Vec<async_channel::Sender<WatchMsg>>,
}

/// A cheaply clonable handle to the [`SharedState`].
///
/// Lock hold times must stay short. Only lookups, inserts and response clones
/// happen under it. Every holder runs on the runtime's own thread on native
/// or the single browser thread on wasm, so contention never involves the
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

impl ServedVault {
    /// Send a frame to every open watch stream. Closed streams drop out.
    pub(crate) fn notify(&mut self, msg: &WatchMsg) {
        self.watchers.retain(|w| w.try_send(msg.clone()).is_ok());
    }
}

impl Shared {
    /// Lock the shared state. A lock poisoned by a panicked peer thread still
    /// yields the data. Content-addressed state cannot be half-written into
    /// an invalid shape.
    pub(crate) fn lock(&self) -> MutexGuard<'_, SharedState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Merge served content into the store. Commit, graph and blob inserts are
/// content-addressed and idempotent. Per-name head upserts let an incoming
/// tip win and report it. Section entries apply per the section's merge
/// policy. See [`gantz_ca::Registry::merge`].
///
/// Every graph and blob is verified against its claimed address before
/// anything is merged. Graphs get a strict re-hash plus a node canonicality
/// check via [`gantz_ca::verify_graph`]. Blobs get a [`gantz_ca::blob_addr`]
/// re-hash. An `Err` leaves the store untouched, so tampered or aliased
/// content is never served. Section entries are advisory metadata with no
/// address to verify.
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

/// The requested objects, where present. Absent objects are skipped. The
/// requester re-requests from another peer or re-heals on the next announce.
///
/// Every answered commit carries its commit-keyed section entries along, for
/// example the commit's stored scene view. A requester cannot know which
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
                objects.extend(keyed_sections(store, Key::Commit(*ca)));
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
            ObjectRef::Closure { tips, have } => {
                closure_objects(store, tips, have, CLOSURE_BUDGET, &mut objects);
            }
        }
    }
    Objects { objects }
}

/// Everything reachable from `tips` that is not reachable from `have`, in
/// [`ObjectRef::Closure`]'s form but whole. A vault push carries this, as a
/// push applies only with its tip's closure complete.
pub fn closure(store: &SessionRegistry, tips: &[CommitAddr], have: &[CommitAddr]) -> Objects {
    let mut objects = Vec::new();
    closure_objects(store, tips, have, usize::MAX, &mut objects);
    Objects { objects }
}

/// Apply an accepted vault change to its served store. Content merges and
/// verifies as in [`merge`]. Each head is set, or removed when `None`. An
/// `Err` leaves the store untouched.
pub fn update_vault(
    store: &mut SessionRegistry,
    heads: &[(Name, Option<CommitAddr>)],
    commits: impl IntoIterator<Item = (CommitAddr, Commit)>,
    graphs: impl IntoIterator<Item = (GraphAddr, DataGraph)>,
    sections: impl IntoIterator<Item = (SectionId, MergePolicy, Liveness, Key, Value)>,
    blobs: impl IntoIterator<Item = (SectionId, BlobLiveness, ContentAddr, Bytes)>,
) -> Result<(), VerifyError> {
    merge(store, [], commits, graphs, sections, blobs)?;
    for (name, head) in heads {
        match head {
            Some(ca) => {
                store.set_head(name.clone(), *ca);
            }
            None => {
                store.remove_head(name);
            }
        }
    }
    Ok(())
}

/// The whole store as a join snapshot. Every head, commit, graph, blob and
/// non-head section entry. Heads travel in the dedicated head list.
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

/// Answer an [`ObjectRef::Closure`] into `objects`.
///
/// Commits go newest-first from `tips`. Each is followed by its keyed
/// sections and the graphs and blobs it newly reaches, so the answer holds
/// whole commits. It stops at the first commit boundary past `budget` bytes
/// of graph and blob content, and the requester asks again for the frontier.
fn closure_objects(
    store: &SessionRegistry,
    tips: &[CommitAddr],
    have: &[CommitAddr],
    budget: usize,
    objects: &mut Vec<Object>,
) {
    let diff = gantz_ca::closure_diff(store, tips.iter().copied(), have.iter().copied());
    let mut size = 0;
    let mut queue: VecDeque<CommitAddr> = tips.iter().copied().collect();
    let mut visited = HashSet::new();
    let mut sent_graphs = HashSet::new();
    let mut sent_blobs = HashSet::new();
    while let Some(ca) = queue.pop_front() {
        if size >= budget {
            break;
        }
        if !diff.commits.contains(&ca) || !visited.insert(ca) {
            continue;
        }
        let Some(commit) = store.commits().get(&ca) else {
            continue;
        };
        objects.push(Object::Commit(ca, commit.clone().into()));
        objects.extend(keyed_sections(store, Key::Commit(ca)));
        let mut graphs = VecDeque::from([commit.graph]);
        while let Some(ga) = graphs.pop_front() {
            if !diff.graphs.contains(&ga) || !sent_graphs.insert(ga) {
                continue;
            }
            let Some(graph) = store.graph(&ga) else {
                continue;
            };
            let blob = proto::encode_graph(graph);
            size += blob.len();
            objects.push(Object::Graph(ga, blob));
            objects.extend(keyed_sections(store, Key::Graph(ga)));
            let out = gantz_ca::data_graph_out(graph);
            graphs.extend(out.graphs);
            for (section, addr) in out.blobs {
                if !diff.blob_live(&section, &addr) || !sent_blobs.insert((section.clone(), addr)) {
                    continue;
                }
                let Some(blobs) = store.blobs().get(&section) else {
                    continue;
                };
                let Some(bytes) = blobs.get(&addr) else {
                    continue;
                };
                size += bytes.len();
                objects.push(Object::Blob {
                    section,
                    liveness: blobs.liveness,
                    addr,
                    bytes: bytes.to_vec(),
                });
            }
        }
        queue.extend(commit.parents());
    }
}

/// Every non-head section entry under the given key.
fn keyed_sections(store: &SessionRegistry, key: Key) -> impl Iterator<Item = Object> + '_ {
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

    /// Content offered under a claimed address it does not verify against is
    /// rejected outright, and the store is left untouched.
    #[test]
    fn merge_rejects_unverified_content() {
        let (mut store, _root_ca, _tip_ca, ga1) = test_store();
        // Honest content for `ga1`, then tampered with an extra node the
        // claimed address does not cover.
        let mut tampered = store.graph(&ga1).unwrap().clone();
        tampered.add_node(NodeData::new("evil", Datum::Map(vec![])));
        let tampered_ga = gantz_ca::graph_addr(&tampered);
        let commit = Commit::new(Duration::from_secs(3), None, ga1);
        let commit_ca = commit_addr(&commit);
        // A graph whose nodes are not in canonical form aliases the same
        // logical content under a second address. It hashes consistently with
        // itself, so the canonicality check, not the hash, must reject it.
        let mut non_canonical = DataGraph::default();
        non_canonical.add_node(NodeData::new(
            "test",
            Datum::Map(vec![
                ("b".to_string(), Datum::Null),
                ("a".to_string(), Datum::Bool(true)),
            ]),
        ));
        let non_canonical_ga = gantz_ca::graph_addr(&non_canonical);
        let blob_claimed = blob_addr(b"pcm");
        let cases = [
            (
                "tampered graph content",
                vec![(name("jam"), commit_ca)],
                vec![(commit_ca, commit)],
                vec![(ga1, tampered)],
                vec![],
                VerifyError::Graph {
                    claimed: ga1,
                    actual: tampered_ga,
                },
            ),
            (
                "non-canonical graph",
                vec![],
                vec![],
                vec![(non_canonical_ga, non_canonical)],
                vec![],
                VerifyError::NonCanonicalNode {
                    graph: non_canonical_ga,
                    node_ix: 0,
                },
            ),
            (
                "tampered blob",
                vec![],
                vec![],
                vec![],
                vec![(
                    "dsp.buffer".to_string(),
                    BlobLiveness::ContentReferenced,
                    blob_claimed,
                    Bytes::from(&b"tampered"[..]),
                )],
                VerifyError::Blob {
                    claimed: blob_claimed,
                    actual: blob_addr(b"tampered"),
                },
            ),
        ];
        let before = store.clone();
        let graph_addrs = |s: &SessionRegistry| {
            s.graphs()
                .keys()
                .copied()
                .collect::<std::collections::HashSet<_>>()
        };
        for (label, heads, commits, graphs, blobs, expected) in cases {
            let err = merge(&mut store, heads, commits, graphs, [], blobs).unwrap_err();
            assert_eq!(err, expected, "{label}");
            assert_eq!(store.commits(), before.commits(), "{label}: commits");
            assert_eq!(graph_addrs(&store), graph_addrs(&before), "{label}: graphs");
            assert_eq!(
                store.sections(),
                before.sections(),
                "{label}: heads, sections"
            );
            assert_eq!(store.blobs(), before.blobs(), "{label}: blobs");
        }
    }

    #[test]
    fn objects_serves_present_refs_and_skips_absent_ones() {
        let (mut store, root_ca, tip_ca, ga1) = test_store();
        let blob = store.add_blob("dsp.buffer", BlobLiveness::ContentReferenced, &b"pcm"[..]);
        let (id, policy, liveness, key, value) = view_entry(tip_ca);
        store.set_section_value(id.clone(), policy, liveness, key.clone(), value.clone());
        let absent = ContentAddr::from([9; 32]);
        let want = Want {
            refs: vec![
                ObjectRef::Commit(root_ca),
                ObjectRef::Commit(CommitAddr::from(absent)),
                ObjectRef::Graph(ga1),
                ObjectRef::Graph(GraphAddr::from(absent)),
                ObjectRef::Blob {
                    section: "dsp.buffer".to_string(),
                    addr: blob,
                },
                ObjectRef::Blob {
                    section: "dsp.buffer".to_string(),
                    addr: absent,
                },
                ObjectRef::Blob {
                    section: "ui.assets".to_string(),
                    addr: absent,
                },
                ObjectRef::Section {
                    id: id.clone(),
                    key: key.clone(),
                },
                ObjectRef::Section {
                    id: id.clone(),
                    key: Key::Commit(CommitAddr::from(absent)),
                },
            ],
        };
        let objects = objects(&store, &want).objects;
        assert_eq!(
            objects,
            vec![
                Object::Commit(root_ca, store.commits()[&root_ca].clone().into()),
                Object::Graph(ga1, proto::encode_graph(store.graph(&ga1).unwrap())),
                Object::Blob {
                    section: "dsp.buffer".to_string(),
                    liveness: BlobLiveness::ContentReferenced,
                    addr: blob,
                    bytes: b"pcm".to_vec(),
                },
                Object::Section {
                    id,
                    policy,
                    liveness,
                    key,
                    value: proto::encode_value(&value),
                },
            ]
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
        // With KeepExisting a differing incoming entry does not clobber.
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

    /// A wanted commit carries its commit-keyed section entries along. See
    /// [`objects`].
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

    /// A chain of `len` commits, each over its own one-node graph. Returns
    /// the commits oldest-first.
    fn chain(store: &mut SessionRegistry, len: u64) -> Vec<CommitAddr> {
        let mut commits = Vec::new();
        for i in 0..len {
            let (ga, g) = graph(&format!("g{i}"));
            let ca = store.commit_graph(Duration::from_secs(i), commits.last().copied(), ga, || g);
            commits.push(ca);
        }
        commits
    }

    fn closure(tips: &[CommitAddr], have: &[CommitAddr]) -> Want {
        Want {
            refs: vec![ObjectRef::Closure {
                tips: tips.to_vec(),
                have: have.to_vec(),
            }],
        }
    }

    /// The commit addresses of the objects, in order.
    fn commit_order(objects: &[Object]) -> Vec<CommitAddr> {
        objects
            .iter()
            .filter_map(|o| match o {
                Object::Commit(ca, _) => Some(*ca),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn closure_answers_newest_first_and_skips_what_the_requester_has() {
        let mut store = SessionRegistry::default();
        let c = chain(&mut store, 4);
        let answer = objects(&store, &closure(&[c[3]], &[c[1]])).objects;
        assert_eq!(commit_order(&answer), vec![c[3], c[2]]);
        let graphs = answer
            .iter()
            .filter(|o| matches!(o, Object::Graph(..)))
            .count();
        assert_eq!(graphs, 2);
        // An unknown `have` cuts nothing.
        let unknown = CommitAddr::from(ContentAddr::from([9; 32]));
        let all = objects(&store, &closure(&[c[3]], &[unknown])).objects;
        assert_eq!(commit_order(&all), vec![c[3], c[2], c[1], c[0]]);
    }

    #[test]
    fn closure_cuts_at_a_commit_boundary_past_the_budget() {
        let mut store = SessionRegistry::default();
        let c = chain(&mut store, 3);
        let mut first = Vec::new();
        closure_objects(&store, &[c[2]], &[], 1, &mut first);
        // Only the tip fits. Its graph comes with it.
        assert_eq!(commit_order(&first), vec![c[2]]);
        assert!(first.iter().any(|o| matches!(o, Object::Graph(..))));
        // The requester continues from the frontier it still misses.
        let mut rest = Vec::new();
        closure_objects(&store, &[c[1]], &[], usize::MAX, &mut rest);
        assert_eq!(commit_order(&rest), vec![c[1], c[0]]);
    }

    #[test]
    fn closure_carries_blobs_and_commit_sections() {
        let mut store = SessionRegistry::default();
        let pcm = store.add_blob("dsp.buffer", BlobLiveness::ContentReferenced, &b"pcm"[..]);
        let mut node = NodeData::new("sampler", Datum::Map(vec![]));
        node.blobs = vec![("dsp.buffer".to_string(), pcm)];
        node.canonicalize();
        let mut g = DataGraph::default();
        g.add_node(node);
        let ga = gantz_ca::graph_addr(&g);
        let ca = store.commit_graph(Duration::from_secs(1), None, ga, || g);
        let (id, policy, liveness, key, value) = view_entry(ca);
        store.set_section_value(id, policy, liveness, key, value);
        let objects = objects(&store, &closure(&[ca], &[])).objects;
        assert!(objects.iter().any(|o| matches!(
            o,
            Object::Blob { addr, .. } if *addr == pcm
        )));
        assert!(objects.iter().any(|o| matches!(
            o,
            Object::Section { key: Key::Commit(k), .. } if *k == ca
        )));
    }

    #[test]
    fn update_vault_sets_and_removes_heads() {
        let (mut store, root_ca, tip_ca, _ga1) = test_store();
        let heads = [(name("jam"), None), (name("riff"), Some(root_ca))];
        update_vault(&mut store, &heads, [], [], [], []).unwrap();
        assert_eq!(store.head(&name("jam")), None);
        assert_eq!(store.head(&name("riff")), Some(root_ca));
        // Content outlives its head.
        assert!(store.commits().contains_key(&tip_ca));
    }
}
