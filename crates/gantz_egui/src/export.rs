//! Export/import helpers for sharing node sets between gantz instances.
//!
//! An export is a [`gantz_ca::Registry`] subset: all GUI metadata (views,
//! demos, descriptions) rides the registry's sections, so no side-band bundle
//! type is needed. Serialization uses the `.gantz` S-expression text format
//! (see [`crate::format`]) under the `.gantz` file extension.

use crate::node::NodeCodec;
use gantz_ca::{DataGraph, GraphAddr, Name};
use gantz_core::node;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

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

/// Parse the raw bytes of a `.gantz` file into a registry.
///
/// The file is the `.gantz` S-expression text format (see [`crate::format`]).
/// Graphs the document does not commit explicitly (hand-authored graphs with no
/// `(commits ...)` entry) are stamped with the current time. Use
/// [`parse_export_at`] to stamp them with a fixed timestamp instead.
pub fn parse_export(
    bytes: &[u8],
    codec: &NodeCodec,
) -> Result<gantz_ca::Registry, ParseExportError> {
    parse_export_at(bytes, now(), codec)
}

/// Like [`parse_export`], but stamps uncommitted (hand-authored) graphs with the
/// given timestamp rather than the current time.
///
/// A fixed timestamp makes the resulting commit addresses reproducible across
/// loads. This matters for content that is re-parsed and whose commits should
/// line up with an already-loaded registry - e.g. the baked-in base, which is
/// parsed both at startup and on demo reset.
pub fn parse_export_at(
    bytes: &[u8],
    now: gantz_ca::Timestamp,
    codec: &NodeCodec,
) -> Result<gantz_ca::Registry, ParseExportError> {
    let text = std::str::from_utf8(bytes).map_err(ParseExportError::Utf8)?;
    crate::format::from_str(text, now, codec).map_err(ParseExportError::Format)
}

/// Like [`parse_export_at`], resolving names the document does not define
/// through `seed` (externally-known name -> head graph associations). Lets a
/// base source reference graphs another source defines - see
/// [`gantz_format::from_str_seeded`].
pub fn parse_export_seeded_at(
    bytes: &[u8],
    now: gantz_ca::Timestamp,
    seed: &std::collections::BTreeMap<String, GraphAddr>,
    codec: &NodeCodec,
) -> Result<gantz_ca::Registry, ParseExportError> {
    let text = std::str::from_utf8(bytes).map_err(ParseExportError::Utf8)?;
    crate::format::from_str_seeded(text, now, seed, codec).map_err(ParseExportError::Format)
}

/// The current time as a [`gantz_ca::Timestamp`] (duration since the Unix epoch).
fn now() -> gantz_ca::Timestamp {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .unwrap_or_default()
}

/// The unique root name of an exported registry, if it has exactly one.
///
/// A name is "root" when no stored graph's [`gantz_ca::NodeData::refs`]
/// column points at the name's head graph: a pure data walk, so no node
/// lookups are needed.
pub fn unique_root_name(registry: &gantz_ca::Registry) -> Option<Name> {
    let referenced: HashSet<GraphAddr> = registry
        .graphs()
        .values()
        .flat_map(|g| g.node_weights())
        .flat_map(|n| n.refs.iter().copied().map(GraphAddr::from))
        .collect();
    let mut roots = registry.heads().filter(|(_, ca)| {
        registry
            .commits()
            .get(ca)
            .is_none_or(|commit| !referenced.contains(&commit.graph))
    });
    let root = roots.next()?;
    roots.next().is_none().then(|| root.0.clone())
}

/// The registry subset transitively reachable from ONLY the given heads,
/// walked over the stored graphs' structural refs/blobs columns (a pure data
/// walk, no node lookups).
fn export_heads_registry(
    registry: &gantz_ca::Registry,
    heads: impl IntoIterator<Item = impl std::borrow::Borrow<gantz_ca::Head>>,
) -> gantz_ca::Registry {
    let seeds = heads
        .into_iter()
        .filter_map(|head| registry.head_commit_ca(head.borrow()));
    let live = gantz_ca::closure_from(registry, seeds);
    gantz_ca::export(registry, &live)
}

/// Serialize an export for the given heads as `.gantz` text.
///
/// Covers both export-head and export-all-named: the export contains the heads'
/// transitively required content along with their views, demos and
/// descriptions. File IO stays with the caller.
pub fn export_heads_sexpr(
    registry: &gantz_ca::Registry,
    heads: impl IntoIterator<Item = impl std::borrow::Borrow<gantz_ca::Head>>,
    codec: &NodeCodec,
) -> Result<String, crate::format::FormatError> {
    let export_registry = export_heads_registry(registry, heads);
    crate::format::to_string(&export_registry, codec)
}

/// As [`export_heads_sexpr`], but serializes in the inline-name format (see
/// [`crate::format::to_string_named`]): graphs named inline, no commits/names
/// tables, references by name. Used for the baked-in base so its file stays
/// hand-editable and free of churning addresses.
pub fn export_heads_sexpr_named(
    registry: &gantz_ca::Registry,
    heads: impl IntoIterator<Item = impl std::borrow::Borrow<gantz_ca::Head>>,
    codec: &NodeCodec,
) -> Result<String, crate::format::FormatError> {
    let export_registry = export_heads_registry(registry, heads);
    crate::format::to_string_named(&export_registry, codec)
}

/// As [`export_heads_sexpr_named`], but exports EXACTLY the given names with
/// no transitive dependency closure: references to graphs outside the set are
/// written by name only, without their `(graph ...)` blocks.
///
/// Used for per-source base write-back, where a source's file must contain
/// only its own graphs - refs into other sources stay by name, and loading
/// resolves them through the seeded parse (see [`parse_export_seeded_at`]).
pub fn export_names_sexpr_named(
    registry: &gantz_ca::Registry,
    names: impl IntoIterator<Item = impl AsRef<str>>,
    codec: &NodeCodec,
) -> Result<String, crate::format::FormatError> {
    let requested: HashSet<Name> = names
        .into_iter()
        .map(|name| name.as_ref().parse().expect("infallible"))
        .collect();
    let mut live = gantz_ca::LiveSet::default();
    for name in &requested {
        let Some(head_ca) = registry.head(name) else {
            continue;
        };
        let Some(commit) = registry.commits().get(&head_ca) else {
            continue;
        };
        live.commits.insert(head_ca);
        live.graphs.insert(commit.graph);
    }
    let mut export_registry = gantz_ca::export(registry, &live);
    // The export keeps every head whose commit survives - identical graphs
    // across sources share commits, so a foreign name could ride along.
    // Restrict to exactly the requested names (their `WithName` metadata,
    // descriptions included, drops with them).
    let extra: Vec<Name> = export_registry
        .heads()
        .filter(|(name, _)| !requested.contains(name))
        .map(|(name, _)| name.clone())
        .collect();
    for name in extra {
        export_registry.remove_head(&name);
    }
    crate::format::to_string_named(&export_registry, codec)
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
pub struct Copied {
    /// Registry dependencies referenced by copied nodes (e.g. Ref nodes),
    /// along with the heads (and their metadata) naming them.
    pub registry: gantz_ca::Registry,
    /// The subgraph of selected nodes and their internal edges, in the
    /// stored data form.
    pub graph: DataGraph,
    /// Positions of nodes in the subgraph.
    pub positions: egui_graph::Layout,
}

/// Build a [`Copied`] payload from the selected nodes in a graph.
///
/// The payload registry carries the transitive closure of the graphs the
/// selected nodes reference, plus the heads (and `WithName`/`WithCommit`
/// metadata) whose tips point at those graphs, so pasting into another
/// registry restores names and views.
pub fn copy(
    registry: &gantz_ca::Registry,
    graph: &DataGraph,
    selected: &HashSet<node::graph::NodeIx>,
    layout: &egui_graph::Layout,
) -> Copied {
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

    // Collect registry deps transitively: the graphs the selected nodes
    // reference, and the graphs *those* graphs reference in turn (a nested
    // graph that itself contains nested graphs), so the whole subtree travels
    // with the clipboard. Blob references ride along likewise. The stored
    // refs/blobs columns cover the whole walk - pure data, no node lookups.
    let mut live = gantz_ca::LiveSet::default();
    let mut stack: Vec<GraphAddr> = subgraph
        .node_weights()
        .flat_map(|n| n.refs.iter().copied())
        .map(GraphAddr::from)
        .filter(|ga| registry.graph(ga).is_some())
        .collect();
    for &(ref section, addr) in subgraph.node_weights().flat_map(|n| n.blobs.iter()) {
        live.blobs.entry(section.clone()).or_default().insert(addr);
    }
    while let Some(graph_ca) = stack.pop() {
        if !live.graphs.insert(graph_ca) {
            continue;
        }
        let Some(nested) = registry.graph(&graph_ca) else {
            continue;
        };
        let out = gantz_ca::data_graph_out(nested);
        stack.extend(
            out.graphs
                .into_iter()
                .filter(|dep| registry.graph(dep).is_some()),
        );
        for (section, addr) in out.blobs {
            live.blobs.entry(section).or_default().insert(addr);
        }
    }

    // Include each collected graph's naming heads (tip commits), so
    // paste-merge restores names and the text format's commits table still
    // describes the named graphs.
    live.commits.extend(
        registry
            .heads()
            .filter(|(_, ca)| {
                registry
                    .commits()
                    .get(ca)
                    .is_some_and(|commit| live.graphs.contains(&commit.graph))
            })
            .map(|(_, ca)| ca),
    );

    Copied {
        registry: gantz_ca::export(registry, &live),
        graph: subgraph,
        positions,
    }
}

/// Paste a [`Copied`] payload into a target graph.
///
/// Merges registry dependencies, adds the subgraph nodes/edges, and maps
/// positions with the given offset. Returns the new node indices in the
/// target graph.
pub fn paste(
    registry: &mut gantz_ca::Registry,
    target_graph: &mut DataGraph,
    target_layout: &mut egui_graph::Layout,
    copied: &Copied,
    offset: egui::Vec2,
) -> Vec<node::graph::NodeIx> {
    registry.merge(copied.registry.clone());
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
/// The copied subgraph rides as a graph named `clipboard` - its positions
/// stored as the clipboard commit's view section entry - alongside the
/// registry dependencies, so the whole payload is one ordinary `.gantz`
/// document. [`copied_from_str`] reverses this.
pub fn copied_to_string(
    copied: &Copied,
    codec: &NodeCodec,
) -> Result<String, crate::format::FormatError> {
    // Add the subgraph to the dependency registry as a fresh root commit
    // named `CLIPBOARD_NAME`. A fixed timestamp keeps the payload
    // deterministic.
    let mut registry = copied.registry.clone();
    let g_addr = registry.add_graph(copied.graph.clone());
    let commit_ca = registry.add_commit(gantz_ca::Commit::new(
        std::time::Duration::ZERO,
        None,
        g_addr,
    ));
    registry.set_head(CLIPBOARD_NAME.parse().expect("infallible"), commit_ca);

    // Carry the positions as the clipboard commit's view. The camera is
    // irrelevant for a clipboard payload, so use the default.
    let view = crate::SceneView {
        camera: crate::Camera::default(),
        layout: copied.positions.clone(),
    };
    crate::section::set_view(&mut registry, commit_ca, &view);

    crate::format::to_string(&registry, codec)
}

/// Parse a clipboard payload produced by [`copied_to_string`].
///
/// Splits the `clipboard` graph (and its positions) back out from the registry
/// dependencies.
pub fn copied_from_str(text: &str, codec: &NodeCodec) -> Result<Copied, ParseCopiedError> {
    let registry = crate::format::from_str(text, now(), codec).map_err(ParseCopiedError::Format)?;

    let clipboard: Name = CLIPBOARD_NAME.parse().expect("infallible");
    let clip_ca = registry
        .head(&clipboard)
        .ok_or(ParseCopiedError::NotClipboard)?;
    let graph: DataGraph = registry
        .commit_graph_ref(&clip_ca)
        .ok_or(ParseCopiedError::NotClipboard)?
        .clone();
    let positions = crate::section::view(&registry, &clip_ca)
        .map(|view| view.layout)
        .unwrap_or_default();

    // Everything reachable outside the clipboard commit is a dependency. The
    // export filters heads (and views) to the kept commits, so the
    // `clipboard` name and its view entry drop out with it.
    let dep_commits: Vec<gantz_ca::CommitAddr> = registry
        .commits()
        .keys()
        .copied()
        .filter(|&ca| ca != clip_ca)
        .collect();
    let live = gantz_ca::closure_from(&registry, dep_commits);
    let deps = gantz_ca::export(&registry, &live);

    Ok(Copied {
        registry: deps,
        graph,
        positions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_ca::{Commit, CommitAddr, ContentAddr};
    use std::collections::HashMap;
    use std::time::Duration;

    fn graph_addr(n: u8) -> GraphAddr {
        GraphAddr::from(ContentAddr::from([n; 32]))
    }

    fn commit_addr_raw(n: u8) -> CommitAddr {
        CommitAddr::from(ContentAddr::from([n; 32]))
    }

    fn name(s: &str) -> Name {
        s.parse().unwrap()
    }

    fn test_registry() -> gantz_ca::Registry {
        let ga = graph_addr(1);
        let ca = commit_addr_raw(10);
        let commit = Commit::new(Duration::from_secs(1), None, ga);
        gantz_ca::Registry::from_parts(
            HashMap::from([(ga, gantz_ca::DataGraph::default())]),
            HashMap::from([(ca, commit)]),
            std::collections::BTreeMap::from([(name("alpha"), ca)]),
        )
    }

    #[test]
    fn export_merge_recovers_data() {
        let export = test_registry();
        let mut target = gantz_ca::Registry::default();
        let report = target.merge(export);
        assert_eq!(report.heads_added, vec![name("alpha")]);
        assert!(report.heads_replaced.is_empty());
        let ca = commit_addr_raw(10);
        assert!(target.commits().contains_key(&ca));
        assert_eq!(target.head(&name("alpha")), Some(ca));
    }

    #[test]
    fn merge_keeps_existing_views() {
        let mut registry = test_registry();
        let ca = commit_addr_raw(10);
        let mut existing_view = crate::SceneView::default();
        existing_view
            .layout
            .insert(egui_graph::NodeId(0), Default::default());
        crate::section::set_view(&mut registry, ca, &existing_view);

        let mut incoming = test_registry();
        crate::section::set_view(&mut incoming, ca, &crate::SceneView::default());
        registry.merge(incoming);

        // Existing view (with 1 layout entry) is preserved, not replaced.
        let view = crate::section::view(&registry, &ca).unwrap();
        assert_eq!(view.layout.len(), 1);
    }

    #[test]
    fn merge_keeps_existing_descriptions_and_demos() {
        let mut registry = test_registry();
        crate::section::set_description(&mut registry, name("alpha"), "local".to_string());
        crate::section::set_demo(&mut registry, name("alpha"), "demo-a".to_string());

        let mut incoming = test_registry();
        crate::section::set_description(&mut incoming, name("alpha"), "imported".to_string());
        crate::section::set_demo(&mut incoming, name("alpha"), "demo-b".to_string());
        crate::section::set_description(&mut incoming, name("beta"), "new".to_string());
        registry.merge(incoming);

        assert_eq!(
            crate::section::description(&registry, &name("alpha")).as_deref(),
            Some("local"),
        );
        assert_eq!(
            crate::section::demo(&registry, &name("alpha")).as_deref(),
            Some("demo-a"),
        );
        assert_eq!(
            crate::section::description(&registry, &name("beta")).as_deref(),
            Some("new"),
        );
    }

    /// Copying a `NamedRef` carries the referenced graph, its naming head
    /// and `WithName` metadata through the clipboard text round-trip, with
    /// positions riding the clipboard commit's view section entry.
    #[test]
    fn clipboard_round_trip_carries_positions_and_deps() {
        use crate::test_node::{TestGraph, codec, commit_named, expr, named_ref};

        let mut reg = gantz_ca::Registry::default();
        let mut leaf_g = TestGraph::default();
        leaf_g.add_node(expr("(+ 1 1)"));
        let (_, leaf_ga) = commit_named(&mut reg, Duration::from_secs(1), &leaf_g, &name("leaf"));
        crate::section::set_description(&mut reg, name("leaf"), "a leaf".to_string());

        // The working graph (in data form): a ref to `leaf` plus a plain
        // expr node.
        let mut typed = TestGraph::default();
        let a = typed.add_node(named_ref("leaf", leaf_ga));
        let b = typed.add_node(expr("(+ 2 2)"));
        let working = gantz_core::data::erase(&typed).unwrap();
        let mut layout = egui_graph::Layout::default();
        layout.insert(egui_graph::NodeId(a.index() as u64), egui::pos2(1.0, 2.0));
        layout.insert(egui_graph::NodeId(b.index() as u64), egui::pos2(3.0, 4.0));
        let selected: HashSet<_> = working.node_indices().collect();

        let copied = copy(&reg, &working, &selected, &layout);
        assert!(copied.registry.graph(&leaf_ga).is_some());
        assert!(copied.registry.head(&name("leaf")).is_some());

        let text = copied_to_string(&copied, &codec()).unwrap();
        let back: Copied = copied_from_str(&text, &codec()).unwrap();

        // The subgraph and its positions survive.
        assert_eq!(back.graph.node_count(), 2);
        for ix in [a, b] {
            let id = egui_graph::NodeId(ix.index() as u64);
            assert_eq!(back.positions.get(&id), copied.positions.get(&id));
        }

        // The deps registry restores the referenced graph, its name and
        // metadata, and carries no clipboard head.
        assert!(back.registry.graph(&leaf_ga).is_some());
        assert!(back.registry.head(&name("leaf")).is_some());
        assert_eq!(
            crate::section::description(&back.registry, &name("leaf")).as_deref(),
            Some("a leaf"),
        );
        assert!(
            back.registry
                .head(&CLIPBOARD_NAME.parse().unwrap())
                .is_none()
        );

        // Pasting merges the deps so the ref resolves in the target.
        let mut target_reg = gantz_ca::Registry::default();
        let mut target_graph = DataGraph::default();
        let mut target_layout = egui_graph::Layout::default();
        let new = paste(
            &mut target_reg,
            &mut target_graph,
            &mut target_layout,
            &back,
            egui::vec2(10.0, 10.0),
        );
        assert_eq!(new.len(), 2);
        assert!(target_reg.graph(&leaf_ga).is_some());
        assert!(target_reg.head(&name("leaf")).is_some());
        assert_eq!(
            target_layout.get(&egui_graph::NodeId(new[0].index() as u64)),
            Some(&egui::pos2(11.0, 12.0)),
        );
    }

    /// Exporting heads as text carries transitive deps and sections through
    /// a parse + merge into a fresh registry.
    #[test]
    fn export_heads_text_round_trip() {
        use crate::test_node::{TestGraph, codec, commit_named, expr, named_ref};

        let mut reg = gantz_ca::Registry::default();
        let mut leaf_g = TestGraph::default();
        leaf_g.add_node(expr("(+ 1 1)"));
        let (_, leaf_ga) = commit_named(&mut reg, Duration::from_secs(1), &leaf_g, &name("leaf"));
        let mut root_g = TestGraph::default();
        root_g.add_node(named_ref("leaf", leaf_ga));
        let (root_ca, root_ga) =
            commit_named(&mut reg, Duration::from_secs(2), &root_g, &name("root"));
        crate::section::set_description(&mut reg, name("root"), "the root".to_string());
        let mut view = crate::SceneView::default();
        view.layout
            .insert(egui_graph::NodeId(0), egui::pos2(5.0, 6.0));
        crate::section::set_view(&mut reg, root_ca, &view);

        let heads = [
            gantz_ca::Head::Branch(name("root")),
            gantz_ca::Head::Branch(name("leaf")),
        ];
        let text = export_heads_sexpr(&reg, heads.iter(), &codec()).unwrap();

        let parsed = parse_export_at(text.as_bytes(), Duration::from_secs(9), &codec()).unwrap();
        let mut fresh = gantz_ca::Registry::default();
        let report = fresh.merge(parsed);
        assert_eq!(report.heads_added.len(), 2);
        assert!(fresh.graph(&leaf_ga).is_some());
        assert!(fresh.graph(&root_ga).is_some());
        assert_eq!(fresh.head(&name("root")), Some(root_ca));
        assert_eq!(
            crate::section::description(&fresh, &name("root")).as_deref(),
            Some("the root"),
        );
        let view = crate::section::view(&fresh, &root_ca).expect("view survives");
        assert_eq!(
            view.layout.get(&egui_graph::NodeId(0)).copied(),
            Some(egui::pos2(5.0, 6.0)),
        );
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
