//! Export/import representation for sharing node sets between gantz instances.
//!
//! The [`Export`] type bundles a [`gantz_ca::Registry`] subset with optional
//! [`GraphViews`] layout data. Serialization uses RON with the `.gantz` file
//! extension.

use crate::GraphViews;
use gantz_ca::{CommitAddr, registry::MergeResult};
use gantz_core::node::{self, graph::Graph};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// File extension for gantz export files (without the leading dot).
pub const FILE_EXTENSION: &str = "gantz";

/// A serializable bundle of a registry subset and its associated view state.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Export<G> {
    pub registry: gantz_ca::Registry<G>,
    #[serde(
        default,
        serialize_with = "gantz_ca::serde_sorted::serialize_map_of_maps"
    )]
    pub views: HashMap<CommitAddr, GraphViews>,
    /// Maps commits to their associated demo graph name (a `demo-*` name).
    #[serde(default)]
    pub demos: HashMap<CommitAddr, String>,
}

/// Produce an [`Export`] by filtering views and demos to commits present in the registry.
pub fn export_with<G>(
    registry: gantz_ca::Registry<G>,
    all_views: &HashMap<CommitAddr, GraphViews>,
    all_demos: &HashMap<CommitAddr, String>,
) -> Export<G>
where
    G: Clone,
{
    let commits = registry.commits();
    let filter = |ca: &&CommitAddr| commits.contains_key(ca);
    let views = all_views
        .iter()
        .filter(|(ca, _)| filter(ca))
        .map(|(&ca, v)| (ca, v.clone()))
        .collect();
    let demos = all_demos
        .iter()
        .filter(|(ca, _)| filter(ca))
        .map(|(&ca, v)| (ca, v.clone()))
        .collect();
    Export {
        registry,
        views,
        demos,
    }
}

/// Merge an [`Export`] into an existing registry, views and demos maps.
///
/// Incoming views and demos for new commits are inserted; existing
/// entries for known commits are kept.
pub fn merge_with<G>(
    registry: &mut gantz_ca::Registry<G>,
    views: &mut HashMap<CommitAddr, GraphViews>,
    demos: &mut HashMap<CommitAddr, String>,
    export: Export<G>,
) -> MergeResult {
    let result = registry.merge(export.registry);
    for (ca, v) in export.views {
        views.entry(ca).or_insert(v);
    }
    for (ca, d) in export.demos {
        demos.entry(ca).or_insert(d);
    }
    result
}

/// Derive a default export filename from a [`gantz_ca::Head`].
pub fn default_filename(head: &gantz_ca::Head) -> String {
    match head {
        gantz_ca::Head::Branch(name) => format!("{name}.{FILE_EXTENSION}"),
        gantz_ca::Head::Commit(ca) => format!("{}.{FILE_EXTENSION}", ca.display_short()),
    }
}

/// Check if a path has the `.gantz` extension.
pub fn is_gantz_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case(FILE_EXTENSION))
        .unwrap_or(false)
}

/// Check if an optional path is a `.gantz` file.
///
/// Returns `true` when the path is absent (e.g. on web) so that files without
/// a known path are accepted speculatively.
pub fn is_maybe_gantz(path: Option<&std::path::Path>) -> bool {
    path.map(is_gantz_path).unwrap_or(true)
}

/// Read bytes from an [`egui::DroppedFile`].
///
/// Tries `file.bytes` first (web), then `std::fs::read` from `file.path` (desktop).
pub fn read_dropped_file(file: &egui::DroppedFile) -> Option<Vec<u8>> {
    if let Some(ref bytes) = file.bytes {
        return Some(bytes.to_vec());
    }
    if let Some(ref path) = file.path {
        return std::fs::read(path).ok();
    }
    None
}

/// A serializable clipboard payload for copied graph nodes.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Copied<N> {
    /// Registry dependencies referenced by copied nodes (e.g. Ref nodes).
    pub export: Export<Graph<N>>,
    /// The subgraph of selected nodes and their internal edges.
    pub graph: Graph<N>,
    /// Positions of nodes in the subgraph.
    pub positions: egui_graph::Layout,
}

/// Build a [`Copied`] payload from the selected nodes in a graph.
pub fn copy<N>(
    registry: &gantz_ca::Registry<Graph<N>>,
    all_views: &HashMap<CommitAddr, GraphViews>,
    graph: &Graph<N>,
    selected: &HashSet<node::graph::NodeIx>,
    layout: &egui_graph::Layout,
) -> Copied<N>
where
    N: Clone + gantz_core::Node,
{
    let subgraph = gantz_core::graph::extract_subgraph(graph, selected);

    // Build positions: iterate selected nodes in sorted order (matching
    // extract_subgraph's deterministic order) alongside new node indices.
    let mut positions = egui_graph::Layout::default();
    let sorted: std::collections::BTreeSet<_> = selected.iter().copied().collect();
    for (old_ix, new_ix) in sorted.iter().zip(subgraph.node_indices()) {
        let old_id = egui_graph::NodeId(old_ix.index() as u64);
        let new_id = egui_graph::NodeId(new_ix.index() as u64);
        if let Some(&pos) = layout.get(&old_id) {
            positions.insert(new_id, pos);
        }
    }

    // Collect registry deps: gather ContentAddrs from nodes, convert to
    // CommitAddrs, filter to those present in the registry, then export.
    let mut required_commits = HashSet::new();
    for n in subgraph.node_indices() {
        for ca in subgraph[n].required_addrs() {
            let commit_ca = CommitAddr::from(ca);
            if registry.commits().contains_key(&commit_ca) {
                required_commits.insert(commit_ca);
            }
        }
    }
    let export_registry = registry.export(&required_commits);
    let export = export_with(export_registry, all_views, &HashMap::new());

    Copied {
        export,
        graph: subgraph,
        positions,
    }
}

/// Paste a [`Copied`] payload into a target graph.
///
/// Merges registry dependencies, adds the subgraph nodes/edges, and maps
/// positions with the given offset. Returns the new node indices in the
/// target graph.
pub fn paste<N>(
    registry: &mut gantz_ca::Registry<Graph<N>>,
    views: &mut HashMap<CommitAddr, GraphViews>,
    demos: &mut HashMap<CommitAddr, String>,
    target_graph: &mut Graph<N>,
    target_layout: &mut egui_graph::Layout,
    copied: &Copied<N>,
    offset: egui::Vec2,
) -> Vec<node::graph::NodeIx>
where
    N: Clone,
{
    merge_with(registry, views, demos, copied.export.clone());
    let new_indices = gantz_core::graph::add_subgraph(target_graph, &copied.graph);

    // Map positions from subgraph indices to target indices with offset.
    for (sub_ix, &target_ix) in copied.graph.node_indices().zip(new_indices.iter()) {
        let sub_id = egui_graph::NodeId(sub_ix.index() as u64);
        let target_id = egui_graph::NodeId(target_ix.index() as u64);
        if let Some(&pos) = copied.positions.get(&sub_id) {
            target_layout.insert(target_id, pos + offset);
        }
    }

    new_indices
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_ca::{Commit, ContentAddr};
    use std::{collections::BTreeMap, time::Duration};

    fn graph_addr(n: u8) -> gantz_ca::GraphAddr {
        gantz_ca::GraphAddr::from(ContentAddr::from([n; 32]))
    }

    fn commit_addr_raw(n: u8) -> CommitAddr {
        CommitAddr::from(ContentAddr::from([n; 32]))
    }

    fn test_export() -> Export<String> {
        let ga = graph_addr(1);
        let ca = commit_addr_raw(10);
        let commit = Commit::new(Duration::from_secs(1), None, ga);
        let registry = gantz_ca::Registry::new(
            HashMap::from([(ga, "graph_a".to_string())]),
            HashMap::from([(ca, commit)]),
            BTreeMap::from([("alpha".to_string(), ca)]),
        );
        Export {
            registry,
            views: HashMap::new(),
            demos: HashMap::new(),
        }
    }

    #[test]
    fn round_trip_serde() {
        let export = test_export();
        let s = ron::to_string(&export).expect("serialize");
        let recovered: Export<String> = ron::from_str(&s).expect("deserialize");
        assert_eq!(
            export.registry.commits().len(),
            recovered.registry.commits().len()
        );
        assert_eq!(
            export.registry.graphs().len(),
            recovered.registry.graphs().len()
        );
        assert_eq!(
            export.registry.names().len(),
            recovered.registry.names().len()
        );
        // Verify the content survived the round-trip.
        let ca = commit_addr_raw(10);
        assert!(recovered.registry.commits().contains_key(&ca));
        assert_eq!(recovered.registry.names().get("alpha"), Some(&ca));
    }

    #[test]
    fn round_trip_export_merge_recovers_data() {
        let export = test_export();
        let s = ron::to_string(&export).expect("serialize");
        let recovered: Export<String> = ron::from_str(&s).expect("deserialize");
        let mut target = gantz_ca::Registry::<String>::default();
        let mut views = HashMap::new();
        let mut demos = HashMap::new();
        let result = merge_with(&mut target, &mut views, &mut demos, recovered);
        assert_eq!(result.names_added, vec!["alpha".to_string()]);
        assert!(result.names_replaced.is_empty());
        let ca = commit_addr_raw(10);
        assert!(target.commits().contains_key(&ca));
        assert_eq!(target.names().get("alpha"), Some(&ca));
    }

    #[test]
    fn export_with_filters_views() {
        let ga = graph_addr(1);
        let ca = commit_addr_raw(10);
        let cb = commit_addr_raw(20);
        let commit = Commit::new(Duration::from_secs(1), None, ga);
        let registry = gantz_ca::Registry::new(
            HashMap::from([(ga, "g".to_string())]),
            HashMap::from([(ca, commit)]),
            BTreeMap::new(),
        );
        let mut all_views = HashMap::new();
        all_views.insert(ca, GraphViews::new());
        all_views.insert(cb, GraphViews::new()); // cb not in registry
        let export = export_with(registry, &all_views, &HashMap::new());
        assert!(export.views.contains_key(&ca));
        assert!(!export.views.contains_key(&cb));
    }

    #[test]
    fn copied_round_trip_serde() {
        use gantz_core::Edge;

        let mut graph: Graph<String> = Graph::default();
        let a = graph.add_node("A".to_string());
        let b = graph.add_node("B".to_string());
        graph.add_edge(a, b, Edge::new(0.into(), 0.into()));

        let mut positions = egui_graph::Layout::default();
        positions.insert(egui_graph::NodeId(0), egui::pos2(10.0, 20.0));
        positions.insert(egui_graph::NodeId(1), egui::pos2(30.0, 40.0));

        let copied = Copied {
            export: Export::default(),
            graph,
            positions,
        };

        let s = ron::to_string(&copied).expect("serialize");
        let recovered: Copied<String> = ron::from_str(&s).expect("deserialize");

        assert_eq!(recovered.graph.node_count(), 2);
        assert_eq!(recovered.graph.edge_count(), 1);
        assert_eq!(recovered.positions.len(), 2);
        assert_eq!(
            recovered.positions[&egui_graph::NodeId(0)],
            egui::pos2(10.0, 20.0),
        );
        assert_eq!(
            recovered.positions[&egui_graph::NodeId(1)],
            egui::pos2(30.0, 40.0),
        );
    }

    #[test]
    fn merge_with_keeps_existing_views() {
        let ga = graph_addr(1);
        let ca = commit_addr_raw(10);
        let commit = Commit::new(Duration::from_secs(1), None, ga);
        let mut registry = gantz_ca::Registry::new(
            HashMap::from([(ga, "g".to_string())]),
            HashMap::from([(ca, commit.clone())]),
            BTreeMap::new(),
        );
        let mut existing_view = GraphViews::new();
        existing_view.insert(vec![0], egui_graph::View::default());
        let mut views = HashMap::from([(ca, existing_view)]);
        let mut demos = HashMap::new();
        let export = Export {
            registry: gantz_ca::Registry::new(
                HashMap::from([(ga, "g".to_string())]),
                HashMap::from([(ca, commit)]),
                BTreeMap::new(),
            ),
            views: HashMap::from([(ca, GraphViews::new())]),
            demos: HashMap::new(),
        };
        merge_with(&mut registry, &mut views, &mut demos, export);
        // Existing view (with 1 entry) should be preserved, not replaced by empty.
        assert_eq!(views[&ca].len(), 1);
    }

    #[test]
    fn is_gantz_path_matches_extension() {
        use std::path::Path;
        assert!(is_gantz_path(Path::new("foo.gantz")));
        assert!(is_gantz_path(Path::new("/tmp/bar.gantz")));
        assert!(is_gantz_path(Path::new("x.GANTZ")));
        assert!(!is_gantz_path(Path::new("foo.txt")));
        assert!(!is_gantz_path(Path::new("foo")));
        assert!(!is_gantz_path(Path::new("gantz")));
    }
}
