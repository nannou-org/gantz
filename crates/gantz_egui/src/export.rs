//! Export/import representation for sharing node sets between gantz instances.
//!
//! The [`Export`] type bundles a [`gantz_ca::Registry`] subset with optional
//! [`egui_graph::View`] layout data. Serialization uses the `.gantz` S-expression text
//! format (see [`crate::format`]) under the `.gantz` file extension.

use gantz_ca::{CaHash, CommitAddr, registry::MergeResult};
use gantz_core::node::{self, GetNode, graph::Graph};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::{HashMap, HashSet};

/// File extension for gantz export files (without the leading dot).
pub const FILE_EXTENSION: &str = "gantz";

/// An error produced when parsing the raw bytes of a `.gantz` file.
#[derive(Debug)]
pub enum ParseExportError {
    Utf8(std::str::Utf8Error),
    /// The S-expression text format failed to parse.
    Format(crate::format::FormatError),
}

impl std::fmt::Display for ParseExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Utf8(e) => write!(f, "invalid UTF-8: {e}"),
            Self::Format(e) => write!(f, "failed to parse .gantz text: {e}"),
        }
    }
}

impl std::error::Error for ParseExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Utf8(e) => Some(e),
            Self::Format(e) => Some(e),
        }
    }
}

/// A serializable bundle of a registry subset and its associated view state.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Export<G> {
    pub registry: gantz_ca::Registry<G>,
    #[serde(default, serialize_with = "gantz_ca::serde_sorted::serialize_map")]
    pub views: HashMap<CommitAddr, egui_graph::View>,
    /// Maps commits to their associated demo graph name (a `demo-*` name).
    #[serde(default)]
    pub demos: HashMap<CommitAddr, String>,
    /// Maps commits to their inlet/outlet documentation.
    #[serde(default)]
    pub interface_docs: HashMap<CommitAddr, crate::InterfaceDocs>,
}

/// Produce an [`Export`] by filtering views, demos and docs to commits present
/// in the registry.
pub fn export_with<G>(
    registry: gantz_ca::Registry<G>,
    all_views: &HashMap<CommitAddr, egui_graph::View>,
    all_demos: &HashMap<CommitAddr, String>,
    all_docs: &HashMap<CommitAddr, crate::InterfaceDocs>,
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
    let interface_docs = all_docs
        .iter()
        .filter(|(ca, _)| filter(ca))
        .map(|(&ca, v)| (ca, v.clone()))
        .collect();
    Export {
        registry,
        views,
        demos,
        interface_docs,
    }
}

/// Parse the raw bytes of a `.gantz` file into an [`Export`].
///
/// The file is the `.gantz` S-expression text format (see [`crate::format`]).
/// Graphs the document does not commit explicitly (hand-authored graphs with no
/// `(commits ...)` entry) are stamped with the current time.
pub fn parse_export<N>(bytes: &[u8]) -> Result<Export<Graph<N>>, ParseExportError>
where
    N: Serialize + DeserializeOwned + CaHash + 'static,
{
    let text = std::str::from_utf8(bytes).map_err(ParseExportError::Utf8)?;
    crate::format::from_str(text, now()).map_err(ParseExportError::Format)
}

/// The current time as a [`gantz_ca::Timestamp`] (duration since the Unix epoch).
fn now() -> gantz_ca::Timestamp {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .unwrap_or_default()
}

/// The unique root name of an export, if it has exactly one.
///
/// `get_node` resolves node lookups outside the export (e.g. builtins).
pub fn unique_root_name<N>(get_node: GetNode, export: &Export<Graph<N>>) -> Option<String>
where
    N: gantz_core::Node,
{
    let mut roots = gantz_core::reg::root_names(get_node, &export.registry);
    (roots.len() == 1).then(|| roots.pop().unwrap())
}

/// Build and serialize an [`Export`] for the given heads as `.gantz` text.
///
/// Covers both export-head and export-all-named: the export contains the heads'
/// transitively required commits along with their views and demos. File IO
/// stays with the caller.
pub fn export_heads_sexpr<N>(
    get_node: GetNode,
    registry: &gantz_ca::Registry<Graph<N>>,
    all_views: &HashMap<CommitAddr, egui_graph::View>,
    all_demos: &HashMap<CommitAddr, String>,
    all_docs: &HashMap<CommitAddr, crate::InterfaceDocs>,
    heads: impl IntoIterator<Item = impl std::borrow::Borrow<gantz_ca::Head>>,
) -> Result<String, crate::format::FormatError>
where
    N: Serialize + DeserializeOwned + gantz_core::Node + Clone,
{
    let export_registry = gantz_core::reg::export_heads(get_node, registry, heads);
    let export = export_with(export_registry, all_views, all_demos, all_docs);
    crate::format::to_string(&export)
}

/// Merge an [`Export`] into an existing registry, views, demos and docs maps.
///
/// Incoming views, demos and docs for new commits are inserted; existing
/// entries for known commits are kept.
pub fn merge_with<G>(
    registry: &mut gantz_ca::Registry<G>,
    views: &mut HashMap<CommitAddr, egui_graph::View>,
    demos: &mut HashMap<CommitAddr, String>,
    docs: &mut HashMap<CommitAddr, crate::InterfaceDocs>,
    export: Export<G>,
) -> MergeResult {
    let result = registry.merge(export.registry);
    for (ca, v) in export.views {
        views.entry(ca).or_insert(v);
    }
    for (ca, d) in export.demos {
        demos.entry(ca).or_insert(d);
    }
    for (ca, d) in export.interface_docs {
        docs.entry(ca).or_insert(d);
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

/// Reserved registry name under which a copied subgraph travels inside a
/// clipboard `.gantz` document (see [`copied_to_string`]).
const CLIPBOARD_NAME: &str = "clipboard";

/// An error produced when parsing a clipboard payload.
#[derive(Debug)]
pub enum ParseCopiedError {
    /// The text was not a valid `.gantz` document.
    Format(crate::format::FormatError),
    /// The document parsed but carried no clipboard graph.
    NotClipboard,
}

impl std::fmt::Display for ParseCopiedError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Format(e) => write!(f, "failed to parse .gantz text: {e}"),
            Self::NotClipboard => write!(f, "document carries no `{CLIPBOARD_NAME}` graph"),
        }
    }
}

impl std::error::Error for ParseCopiedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Format(e) => Some(e),
            Self::NotClipboard => None,
        }
    }
}

/// A clipboard payload for copied graph nodes.
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
    all_views: &HashMap<CommitAddr, egui_graph::View>,
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

    // Collect registry deps transitively: the commits the selected nodes
    // reference, and the commits *those* graphs reference in turn (a nested
    // graph that itself contains nested graphs), so the whole subtree travels
    // with the clipboard.
    let mut required_commits = HashSet::new();
    let mut stack: Vec<CommitAddr> = subgraph
        .node_weights()
        .flat_map(|n| n.required_addrs())
        .map(CommitAddr::from)
        .filter(|ca| registry.commits().contains_key(ca))
        .collect();
    while let Some(commit_ca) = stack.pop() {
        if !required_commits.insert(commit_ca) {
            continue;
        }
        if let Some(nested) = registry.commit_graph_ref(&commit_ca) {
            for ca in nested.node_weights().flat_map(|n| n.required_addrs()) {
                let dep = CommitAddr::from(ca);
                if registry.commits().contains_key(&dep) {
                    stack.push(dep);
                }
            }
        }
    }
    let export_registry = registry.export(&required_commits);
    let export = export_with(export_registry, all_views, &HashMap::new(), &HashMap::new());

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
    views: &mut HashMap<CommitAddr, egui_graph::View>,
    demos: &mut HashMap<CommitAddr, String>,
    docs: &mut HashMap<CommitAddr, crate::InterfaceDocs>,
    target_graph: &mut Graph<N>,
    target_layout: &mut egui_graph::Layout,
    copied: &Copied<N>,
    offset: egui::Vec2,
) -> Vec<node::graph::NodeIx>
where
    N: Clone,
{
    merge_with(registry, views, demos, docs, copied.export.clone());
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

/// Serialize a [`Copied`] payload as a `.gantz` document.
///
/// The copied subgraph rides as a graph named `clipboard` - its positions stored
/// as that graph's layout view - alongside the registry dependencies, so the
/// whole payload is one ordinary `.gantz` document. [`copied_from_str`] reverses
/// this.
pub fn copied_to_string<N>(copied: &Copied<N>) -> Result<String, crate::format::FormatError>
where
    N: Serialize + DeserializeOwned + CaHash + Clone + 'static,
{
    // Add the subgraph to the dependency registry as a fresh root commit named
    // `CLIPBOARD_NAME`. A fixed timestamp keeps the payload deterministic.
    let mut registry = copied.export.registry.clone();
    let g_addr = registry.add_graph(copied.graph.clone());
    let commit_ca = registry.add_commit(gantz_ca::Commit::new(
        std::time::Duration::ZERO,
        None,
        g_addr,
    ));
    registry.insert_name(CLIPBOARD_NAME.to_string(), commit_ca);

    // Carry the positions as the clipboard graph's layout view.
    let mut views = copied.export.views.clone();
    views.insert(
        commit_ca,
        egui_graph::View {
            scene_rect: egui::Rect::ZERO,
            layout: copied.positions.clone(),
        },
    );

    let export = Export {
        registry,
        views,
        demos: copied.export.demos.clone(),
        interface_docs: copied.export.interface_docs.clone(),
    };
    crate::format::to_string(&export)
}

/// Parse a clipboard payload produced by [`copied_to_string`].
///
/// Splits the `clipboard` graph (and its positions) back out from the registry
/// dependencies.
pub fn copied_from_str<N>(text: &str) -> Result<Copied<N>, ParseCopiedError>
where
    N: Serialize + DeserializeOwned + CaHash + Clone + 'static,
{
    let mut export = crate::format::from_str::<N>(text, now()).map_err(ParseCopiedError::Format)?;

    let clip_ca = export
        .registry
        .names()
        .get(CLIPBOARD_NAME)
        .copied()
        .ok_or(ParseCopiedError::NotClipboard)?;
    let graph = export
        .registry
        .commit_graph_ref(&clip_ca)
        .cloned()
        .ok_or(ParseCopiedError::NotClipboard)?;
    let positions = export
        .views
        .get(&clip_ca)
        .map(|view| view.layout.clone())
        .unwrap_or_default();

    // Everything but the clipboard commit is a dependency. `export` filters
    // names to the kept commits, so the `clipboard` name drops out with it.
    let deps: HashSet<CommitAddr> = export
        .registry
        .commits()
        .keys()
        .copied()
        .filter(|&ca| ca != clip_ca)
        .collect();
    let registry = export.registry.export(&deps);
    export.views.remove(&clip_ca);

    Ok(Copied {
        export: Export {
            registry,
            views: export.views,
            demos: export.demos,
            interface_docs: export.interface_docs,
        },
        graph,
        positions,
    })
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
            interface_docs: HashMap::new(),
        }
    }

    #[test]
    fn export_merge_recovers_data() {
        let export = test_export();
        let mut target = gantz_ca::Registry::<String>::default();
        let mut views = HashMap::new();
        let mut demos = HashMap::new();
        let mut docs = HashMap::new();
        let result = merge_with(&mut target, &mut views, &mut demos, &mut docs, export);
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
        all_views.insert(ca, egui_graph::View::default());
        all_views.insert(cb, egui_graph::View::default()); // cb not in registry
        let export = export_with(registry, &all_views, &HashMap::new(), &HashMap::new());
        assert!(export.views.contains_key(&ca));
        assert!(!export.views.contains_key(&cb));
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
        let mut existing_view = egui_graph::View::default();
        existing_view
            .layout
            .insert(egui_graph::NodeId(0), Default::default());
        let mut views = HashMap::from([(ca, existing_view)]);
        let mut demos = HashMap::new();
        let mut docs = HashMap::new();
        let export = Export {
            registry: gantz_ca::Registry::new(
                HashMap::from([(ga, "g".to_string())]),
                HashMap::from([(ca, commit)]),
                BTreeMap::new(),
            ),
            views: HashMap::from([(ca, egui_graph::View::default())]),
            demos: HashMap::new(),
            interface_docs: HashMap::new(),
        };
        merge_with(&mut registry, &mut views, &mut demos, &mut docs, export);
        // Existing view (with 1 layout entry) should be preserved, not replaced.
        assert_eq!(views[&ca].layout.len(), 1);
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
