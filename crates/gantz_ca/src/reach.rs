//! The one reachability walk. Liveness for prune, closure for export and
//! want-lists for sync all derive from here.
//!
//! Edge rules:
//! - A commit contributes its parents and its graph.
//! - A graph contributes its nodes' structural reference columns. See
//!   [`data_graph_out`]. These are nested graph references and blob
//!   references.
//! - Blobs and section values are leaves.
//!
//! Roots are the entries of `Root`-liveness sections, at minimum the `heads`
//! section, plus any extra seeds the caller supplies.
//!
//! Section entry liveness is not part of [`LiveSet`]. It is a pure function
//! of the surviving content, so [`export`] and [`prune`] apply each
//! section's stored [`Liveness`] rule against the filtered registry
//! directly.

use crate::{
    BlobLiveness, CommitAddr, ContentAddr, DataGraph, GraphAddr, Liveness, Registry, SectionId,
    Value,
};
use std::collections::{BTreeMap, HashSet, VecDeque};

/// The outgoing content references of a single graph.
#[derive(Clone, Debug, Default)]
pub struct OutRefs {
    /// Nested graph references. For example, from `Ref` nodes.
    pub graphs: Vec<GraphAddr>,
    /// Blob references, tagged with their blob section.
    pub blobs: Vec<(SectionId, ContentAddr)>,
}

/// The live content of a registry. The closure of the roots and any extra
/// seeds over the edge rules above.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LiveSet {
    pub commits: HashSet<CommitAddr>,
    pub graphs: HashSet<GraphAddr>,
    pub blobs: BTreeMap<SectionId, HashSet<ContentAddr>>,
}

impl LiveSet {
    /// Whether the given blob is live.
    pub fn blob_live(&self, section: &str, addr: &ContentAddr) -> bool {
        self.blobs.get(section).is_some_and(|s| s.contains(addr))
    }
}

/// The outgoing references of a stored data graph. The union of its nodes'
/// structural [`refs`](crate::NodeData::refs) and
/// [`blobs`](crate::NodeData::blobs) columns, sorted and deduplicated.
///
/// A pure data walk, so any peer can compute reachability without the node
/// types compiled in.
pub fn data_graph_out(g: &DataGraph) -> OutRefs {
    let mut graphs: Vec<GraphAddr> = g
        .node_weights()
        .flat_map(|n| n.refs.iter().copied().map(GraphAddr::from))
        .collect();
    graphs.sort();
    graphs.dedup();
    let mut blobs: Vec<(SectionId, ContentAddr)> = g
        .node_weights()
        .flat_map(|n| n.blobs.iter().cloned())
        .collect();
    blobs.sort();
    blobs.dedup();
    OutRefs { graphs, blobs }
}

/// The live closure of `reg` from its `Root`-liveness sections plus the
/// given extra commit seeds.
///
/// Dangling seeds and references are tolerated and not walked.
pub fn closure(reg: &Registry, extra_commits: impl IntoIterator<Item = CommitAddr>) -> LiveSet {
    // Roots are every commit-valued entry of every Root-liveness section.
    let roots = reg
        .sections()
        .values()
        .filter(|section| section.liveness == Liveness::Root)
        .flat_map(|section| section.entries.values())
        .filter_map(|value| match value {
            Value::Commit(ca) => Some(*ca),
            _ => None,
        })
        .collect::<Vec<_>>();
    closure_from(reg, roots.into_iter().chain(extra_commits))
}

/// The live closure of `reg` from the given commit seeds alone, ignoring the
/// registry's own roots.
///
/// For minimal exports of a specific head set. [`closure`] is this plus the
/// `Root`-liveness section seeds.
pub fn closure_from(reg: &Registry, seeds: impl IntoIterator<Item = CommitAddr>) -> LiveSet {
    let mut live = LiveSet::default();
    let mut commit_queue: VecDeque<CommitAddr> = VecDeque::new();
    let mut graph_queue: VecDeque<GraphAddr> = VecDeque::new();
    commit_queue.extend(seeds);

    while let Some(ca) = commit_queue.pop_front() {
        let Some(commit) = reg.commits().get(&ca) else {
            continue;
        };
        if !live.commits.insert(ca) {
            continue;
        }
        commit_queue.extend(commit.parents());
        graph_queue.push_back(commit.graph);
        while let Some(ga) = graph_queue.pop_front() {
            let Some(graph) = reg.graph(&ga) else {
                continue;
            };
            if !live.graphs.insert(ga) {
                continue;
            }
            let out = data_graph_out(graph);
            graph_queue.extend(out.graphs);
            for (section, addr) in out.blobs {
                live.blobs.entry(section).or_default().insert(addr);
            }
        }
    }

    // Blobs referenced by live section entries via `Value::Blob`.
    for section in reg.sections().values() {
        for (key, value) in &section.entries {
            if !entry_live(reg, section.liveness, key, &live) {
                continue;
            }
            if let Value::Blob(blob_section, addr) = value {
                live.blobs
                    .entry(blob_section.clone())
                    .or_default()
                    .insert(*addr);
            }
        }
    }

    // Pinned blob stores are live wholesale.
    for (id, store) in reg.blobs() {
        if store.liveness == BlobLiveness::Pinned {
            live.blobs
                .entry(id.clone())
                .or_default()
                .extend(store.entries.keys().copied());
        }
    }

    live
}

/// Export the live subset of the registry. Content is filtered by `live`.
/// Section entries are filtered by their stored liveness against the
/// exported content. Heads whose commit falls outside the export are
/// dropped, and their `WithName` metadata with them.
pub fn export(reg: &Registry, live: &LiveSet) -> Registry {
    let mut exported = reg.clone();
    prune(&mut exported, live);
    exported
}

/// Prune the registry down to the live set. Dead content is removed. Heads
/// pointing outside the live set are dropped. Each section's entries are
/// filtered by its stored liveness rule against the surviving state.
/// Invalid commit parents are detached. Emptied sections and blob stores
/// are removed.
pub fn prune(reg: &mut Registry, live: &LiveSet) {
    reg.retain_live(live);
}

/// Whether a section entry is live against the given registry and live set.
/// Used by [`closure`]'s blob-reference pass. `Root` entries are treated as
/// live, since their commit values were the walk's seeds.
fn entry_live(reg: &Registry, liveness: Liveness, key: &crate::Key, live: &LiveSet) -> bool {
    use crate::Key;
    match liveness {
        Liveness::Pinned | Liveness::Root => true,
        Liveness::WithName => match key {
            Key::Name(name) => reg.head(name).is_some_and(|ca| live.commits.contains(&ca)),
            _ => false,
        },
        Liveness::WithCommit => match key {
            Key::Commit(ca) => live.commits.contains(ca),
            _ => false,
        },
        Liveness::WithGraph => match key {
            Key::Graph(ga) => live.graphs.contains(ga),
            _ => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ContentAddr, Datum, Key, MergePolicy, Name, NodeData, registry::section_insert_datum,
    };
    use std::collections::BTreeMap;
    use std::time::Duration;

    fn name(s: &str) -> Name {
        s.parse().unwrap()
    }

    /// A node with the given structural reference columns.
    fn node(refs: Vec<GraphAddr>, blobs: Vec<(SectionId, ContentAddr)>) -> NodeData {
        let mut n = NodeData::new("test", Datum::Map(vec![]));
        n.refs = refs.into_iter().map(Into::into).collect();
        n.blobs = blobs;
        n.canonicalize();
        n
    }

    /// Add a one-node graph carrying the given reference columns.
    fn add_graph(
        reg: &mut Registry,
        refs: Vec<GraphAddr>,
        blobs: Vec<(SectionId, ContentAddr)>,
    ) -> GraphAddr {
        let mut g = DataGraph::default();
        g.add_node(node(refs, blobs));
        reg.add_graph(g)
    }

    #[test]
    fn closure_walks_heads_parents_and_nested_graphs() {
        let mut reg = Registry::default();
        let nested = add_graph(&mut reg, vec![], vec![]);
        let root_ga = add_graph(&mut reg, vec![nested], vec![]);
        let c1 = reg.commit_graph(Duration::from_secs(1), None, root_ga, || unreachable!());
        let c2 = reg.commit_graph(Duration::from_secs(2), Some(c1), root_ga, || unreachable!());
        reg.set_head(name("alpha"), c2);
        // A dead graph whose sole node references a dangling address.
        let dangling = GraphAddr::from(ContentAddr::from([99; 32]));
        let dead_ga = add_graph(&mut reg, vec![dangling], vec![]);
        let _dead = reg.commit_graph(Duration::from_secs(3), None, dead_ga, || unreachable!());
        let live = closure(&reg, []);
        assert!(live.commits.contains(&c1));
        assert!(live.commits.contains(&c2));
        assert!(live.graphs.contains(&root_ga));
        assert!(live.graphs.contains(&nested));
        assert!(!live.graphs.contains(&dead_ga));
        assert_eq!(live.commits.len(), 2);
    }

    #[test]
    fn graphs_stay_live_through_content_refs_without_commits() {
        let mut reg = Registry::default();
        let nested = add_graph(&mut reg, vec![], vec![]);
        let root_ga = add_graph(&mut reg, vec![nested], vec![]);
        let c = reg.commit_graph(Duration::from_secs(1), None, root_ga, || unreachable!());
        reg.set_head(name("alpha"), c);
        let live = closure(&reg, []);
        prune(&mut reg, &live);
        assert!(reg.graph(&nested).is_some());
    }

    #[test]
    fn prune_drops_dead_content_and_sections_follow() {
        let mut reg = Registry::default();
        let ga = add_graph(&mut reg, vec![], vec![]);
        let live_c = reg.commit_graph(Duration::from_secs(1), None, ga, || unreachable!());
        let dead_c = reg.commit_graph(Duration::from_secs(2), None, ga, || unreachable!());
        reg.set_head(name("alpha"), live_c);
        for ca in [live_c, dead_c] {
            section_insert_datum(
                &mut reg,
                "test.view",
                MergePolicy::KeepExisting,
                Liveness::WithCommit,
                Key::Commit(ca),
                &"view".to_string(),
            )
            .unwrap();
        }
        let live = closure(&reg, []);
        prune(&mut reg, &live);
        assert!(reg.commits().contains_key(&live_c));
        assert!(!reg.commits().contains_key(&dead_c));
        let section = reg.section("test.view").unwrap();
        assert!(section.entries.contains_key(&Key::Commit(live_c)));
        assert!(!section.entries.contains_key(&Key::Commit(dead_c)));
    }

    #[test]
    fn prune_detaches_pruned_parents() {
        let mut reg = Registry::default();
        let ga = add_graph(&mut reg, vec![], vec![]);
        let old = reg.commit_graph(Duration::from_secs(1), None, ga, || unreachable!());
        let tip = reg.commit_graph(Duration::from_secs(2), Some(old), ga, || unreachable!());
        reg.set_head(name("alpha"), tip);
        let live = LiveSet {
            commits: [tip].into_iter().collect(),
            graphs: [ga].into_iter().collect(),
            blobs: BTreeMap::new(),
        };
        prune(&mut reg, &live);
        assert!(!reg.commits().contains_key(&old));
        assert_eq!(reg.commits()[&tip].parent, None);
    }

    #[test]
    fn blob_liveness_follows_content_refs() {
        let mut reg = Registry::default();
        let used = reg.add_blob("dsp.buffer", BlobLiveness::ContentReferenced, &b"used"[..]);
        let unused = reg.add_blob(
            "dsp.buffer",
            BlobLiveness::ContentReferenced,
            &b"unused"[..],
        );
        let ga = add_graph(&mut reg, vec![], vec![("dsp.buffer".to_string(), used)]);
        let c = reg.commit_graph(Duration::from_secs(1), None, ga, || unreachable!());
        reg.set_head(name("alpha"), c);
        let live = closure(&reg, []);
        assert!(live.blob_live("dsp.buffer", &used));
        assert!(!live.blob_live("dsp.buffer", &unused));
        prune(&mut reg, &live);
        assert!(reg.blob("dsp.buffer", &used).is_some());
        assert!(reg.blob("dsp.buffer", &unused).is_none());
    }

    #[test]
    fn blob_liveness_follows_section_values() {
        let mut reg = Registry::default();
        let pinned_by_section =
            reg.add_blob("ui.assets", BlobLiveness::SectionReferenced, &b"icon"[..]);
        let orphan = reg.add_blob("ui.assets", BlobLiveness::SectionReferenced, &b"old"[..]);
        let ga = add_graph(&mut reg, vec![], vec![]);
        let c = reg.commit_graph(Duration::from_secs(1), None, ga, || unreachable!());
        reg.set_head(name("alpha"), c);
        reg.set_section_value(
            "test.icons",
            MergePolicy::KeepExisting,
            Liveness::WithName,
            Key::Name(name("alpha")),
            Value::Blob("ui.assets".to_string(), pinned_by_section),
        );
        let live = closure(&reg, []);
        assert!(live.blob_live("ui.assets", &pinned_by_section));
        assert!(!live.blob_live("ui.assets", &orphan));
    }

    #[test]
    fn export_filters_heads_to_exported_commits() {
        let mut reg = Registry::default();
        let ga = add_graph(&mut reg, vec![], vec![]);
        let ca = reg.commit_graph(Duration::from_secs(1), None, ga, || unreachable!());
        let cb = reg.commit_graph(Duration::from_secs(2), None, ga, || unreachable!());
        reg.set_head(name("alpha"), ca);
        reg.set_head(name("beta"), cb);
        // Alpha's closure only.
        let live = LiveSet {
            commits: [ca].into_iter().collect(),
            graphs: [ga].into_iter().collect(),
            blobs: BTreeMap::new(),
        };
        let exported = export(&reg, &live);
        assert_eq!(exported.head(&name("alpha")), Some(ca));
        assert_eq!(exported.head(&name("beta")), None);
        assert!(exported.commits().contains_key(&ca));
        assert!(!exported.commits().contains_key(&cb));
        // The source registry is untouched.
        assert_eq!(reg.head(&name("beta")), Some(cb));
    }

    #[test]
    fn export_keeps_with_name_metadata_for_exported_heads_only() {
        let mut reg = Registry::default();
        let ga = add_graph(&mut reg, vec![], vec![]);
        let ca = reg.commit_graph(Duration::from_secs(1), None, ga, || unreachable!());
        let cb = reg.commit_graph(Duration::from_secs(2), None, ga, || unreachable!());
        reg.set_head(name("alpha"), ca);
        reg.set_head(name("beta"), cb);
        for n in ["alpha", "beta"] {
            section_insert_datum(
                &mut reg,
                "test.description",
                MergePolicy::KeepExisting,
                Liveness::WithName,
                Key::Name(name(n)),
                &format!("doc for {n}"),
            )
            .unwrap();
        }
        let live = LiveSet {
            commits: [ca].into_iter().collect(),
            graphs: [ga].into_iter().collect(),
            blobs: BTreeMap::new(),
        };
        let exported = export(&reg, &live);
        let section = exported.section("test.description").unwrap();
        assert!(section.entries.contains_key(&Key::Name(name("alpha"))));
        assert!(!section.entries.contains_key(&Key::Name(name("beta"))));
    }

    #[test]
    fn unknown_section_survives_export_and_merge() {
        let mut reg = Registry::default();
        let ga = add_graph(&mut reg, vec![], vec![]);
        let ca = reg.commit_graph(Duration::from_secs(1), None, ga, || unreachable!());
        reg.set_head(name("alpha"), ca);
        section_insert_datum(
            &mut reg,
            "laser.palette",
            MergePolicy::KeepExisting,
            Liveness::Pinned,
            Key::Name(name("show")),
            &vec![255u8, 0, 128],
        )
        .unwrap();
        let live = closure(&reg, []);
        let exported = export(&reg, &live);
        assert!(exported.section("laser.palette").is_some());
        let mut other = Registry::default();
        other.merge(exported);
        let section = other.section("laser.palette").unwrap();
        assert_eq!(section.entries.len(), 1);
        assert_eq!(section.liveness, Liveness::Pinned);
    }

    /// The full walk over stored data graphs. Nested graphs and blobs stay
    /// live purely through the structural refs columns.
    #[test]
    fn data_graph_closure_is_a_pure_data_walk() {
        let mut reg = Registry::default();
        let buf = reg.add_blob("dsp.buffer", BlobLiveness::ContentReferenced, &b"pcm"[..]);
        let orphan = reg.add_blob("dsp.buffer", BlobLiveness::ContentReferenced, &b"old"[..]);
        let nested = add_graph(&mut reg, vec![], vec![("dsp.buffer".to_string(), buf)]);
        let root = add_graph(&mut reg, vec![nested], vec![]);
        let dead = {
            let mut g = DataGraph::default();
            g.add_node(node(vec![], vec![("dsp.buffer".to_string(), orphan)]));
            g.add_node(node(vec![], vec![]));
            reg.add_graph(g)
        };
        let c = reg.commit_graph(Duration::from_secs(1), None, root, || unreachable!());
        reg.set_head(name("alpha"), c);

        let live = closure(&reg, []);
        assert!(live.graphs.contains(&root));
        assert!(live.graphs.contains(&nested));
        assert!(!live.graphs.contains(&dead));
        assert!(live.blob_live("dsp.buffer", &buf));
        assert!(!live.blob_live("dsp.buffer", &orphan));
        prune(&mut reg, &live);
        assert!(reg.graph(&dead).is_none());
        assert!(reg.blob("dsp.buffer", &orphan).is_none());
    }

    #[test]
    fn export_of_nothing_is_empty() {
        let mut reg = Registry::default();
        let ga = add_graph(&mut reg, vec![], vec![]);
        let ca = reg.commit_graph(Duration::from_secs(1), None, ga, || unreachable!());
        reg.set_head(name("alpha"), ca);
        let exported = export(&reg, &LiveSet::default());
        assert!(exported.commits().is_empty());
        assert!(exported.graphs().is_empty());
        assert_eq!(exported.head(&name("alpha")), None);
        assert!(exported.sections().is_empty());
    }
}
