//! Export/import helpers for sharing node sets between gantz instances.
//!
//! An export is a [`gantz_ca::Registry`] subset. All GUI metadata rides the
//! registry's sections, so no side-band bundle type is needed. Views, demos
//! and descriptions are that metadata. Serialization uses the `.gantz`
//! S-expression text format from [`crate::format`] under the `.gantz` file
//! extension.

use crate::codec::DataCodec;
use gantz_ca::{DataGraph, GraphAddr, Name};
use gantz_core::node::{self, state};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// File extension for gantz export files, without the leading dot.
pub const FILE_EXTENSION: &str = "gantz";

/// An error produced when parsing the raw bytes of a `.gantz` file.
#[derive(Debug)]
pub enum ParseExportError {
    Utf8(std::str::Utf8Error),
    /// The S-expression text format failed to parse.
    Format(crate::format::FormatError),
}

impl ParseExportError {
    /// Whether the text references a graph name that is not loaded yet.
    pub fn is_missing_dependency(&self) -> bool {
        matches!(
            self,
            Self::Format(e) if matches!(e.kind, gantz_format::ErrorKind::MissingDependency(_))
        )
    }
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
/// The file is the `.gantz` S-expression text format from [`crate::format`].
/// Hand-authored graphs with no `(commits ...)` entry are stamped with the
/// current time. Use [`parse_export_at`] to stamp them with a fixed timestamp
/// instead.
pub fn parse_export(
    bytes: &[u8],
    codec: &DataCodec,
) -> Result<gantz_ca::Registry, ParseExportError> {
    parse_export_at(bytes, now(), codec)
}

/// Like [`parse_export`], but stamps hand-authored graphs with the given
/// timestamp rather than the current time.
///
/// A fixed timestamp makes the resulting commit addresses reproducible across
/// loads. This matters for content that is re-parsed and whose commits should
/// line up with an already-loaded registry. The baked-in base is one example.
/// It is parsed both at startup and on demo reset.
pub fn parse_export_at(
    bytes: &[u8],
    now: gantz_ca::Timestamp,
    codec: &DataCodec,
) -> Result<gantz_ca::Registry, ParseExportError> {
    let text = std::str::from_utf8(bytes).map_err(ParseExportError::Utf8)?;
    crate::format::from_str(text, now, codec).map_err(ParseExportError::Format)
}

/// Like [`parse_export_at`], but resolves names the document does not define
/// through `seed`. The seed maps externally-known names to head graphs. This
/// lets a base source reference graphs another source defines. See
/// [`gantz_format::from_str_normalized`].
pub fn parse_export_seeded_at(
    bytes: &[u8],
    now: gantz_ca::Timestamp,
    seed: &std::collections::BTreeMap<String, GraphAddr>,
    codec: &DataCodec,
) -> Result<gantz_ca::Registry, ParseExportError> {
    let text = std::str::from_utf8(bytes).map_err(ParseExportError::Utf8)?;
    crate::format::from_str_seeded(text, now, seed, codec).map_err(ParseExportError::Format)
}

/// Parse the raw bytes of a `.gantz` file to merge into `registry`.
///
/// A name that the file uses but does not define resolves to the head graph
/// of that name in `registry`. The names that the file defines shadow those
/// names. An old name in `renames` resolves to the head graph of its new
/// name. Each old name in the file then takes its new name.
pub fn parse_import(
    bytes: &[u8],
    now: gantz_ca::Timestamp,
    registry: &gantz_ca::Registry,
    renames: &[(Name, Name)],
    codec: &DataCodec,
) -> Result<gantz_ca::Registry, ParseExportError> {
    let names: crate::base::Names = registry.heads().map(|(n, ca)| (n.clone(), ca)).collect();
    let seed = crate::base::seed_graph_addrs(&names, renames, registry);
    let mut import = parse_export_seeded_at(bytes, now, &seed, codec)?;
    crate::resync::rename(&mut import, now, renames);
    Ok(import)
}

/// The current time as a [`gantz_ca::Timestamp`], the duration since the Unix
/// epoch.
fn now() -> gantz_ca::Timestamp {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .unwrap_or_default()
}

/// The unique root name of an exported registry, if it has exactly one.
///
/// A name is "root" when no stored graph's [`gantz_ca::NodeData::refs`]
/// column points at the name's head graph. This is a pure data walk, so no
/// node lookups are needed.
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

/// The registry subset transitively reachable from only the given heads. The
/// walk reads the stored graphs' structural refs and blobs columns. It is a
/// pure data walk with no node lookups. Node state rides only with a name that
/// keeps it.
fn export_heads_registry(
    registry: &gantz_ca::Registry,
    heads: impl IntoIterator<Item = impl std::borrow::Borrow<gantz_ca::Head>>,
) -> gantz_ca::Registry {
    let seeds = heads
        .into_iter()
        .filter_map(|head| registry.head_commit_ca(head.borrow()));
    let live = gantz_ca::closure_from(registry, seeds);
    let mut export_registry = gantz_ca::export(registry, &live);
    state::retain_kept(&mut export_registry);
    export_registry
}

/// Serialize an export for the given heads as `.gantz` text.
///
/// Covers both export-head and export-all-named. The export contains the heads'
/// transitively required content along with their views, demos and
/// descriptions, and the node state of each name that keeps it. File IO stays
/// with the caller.
pub fn export_heads_sexpr(
    registry: &gantz_ca::Registry,
    heads: impl IntoIterator<Item = impl std::borrow::Borrow<gantz_ca::Head>>,
    codec: &DataCodec,
) -> Result<String, crate::format::FormatError> {
    let export_registry = export_heads_registry(registry, heads);
    crate::format::to_string(&export_registry, codec)
}

/// As [`export_heads_sexpr`], but serializes in the inline-name format. See
/// [`crate::format::to_string_named`]. Used for the baked-in base so its file
/// stays hand-editable and free of churning addresses.
pub fn export_heads_sexpr_named(
    registry: &gantz_ca::Registry,
    heads: impl IntoIterator<Item = impl std::borrow::Borrow<gantz_ca::Head>>,
    codec: &DataCodec,
) -> Result<String, crate::format::FormatError> {
    let export_registry = export_heads_registry(registry, heads);
    crate::format::to_string_named(&export_registry, codec)
}

/// As [`export_heads_sexpr_named`], but exports exactly the given names with
/// no transitive dependency closure. References to graphs outside the set are
/// written by name only, without their `(graph ...)` blocks.
///
/// Used for per-source base write-back, where a source's file must contain
/// only its own graphs. Refs into other sources stay by name. Loading
/// resolves them through the seeded parse in [`parse_export_seeded_at`].
/// Node state rides only with a name that keeps it.
pub fn export_names_sexpr_named(
    registry: &gantz_ca::Registry,
    names: impl IntoIterator<Item = impl AsRef<str>>,
    codec: &DataCodec,
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
    // The export keeps every head whose commit survives. Identical graphs
    // across sources share commits, so a foreign name could ride along.
    // Restrict to exactly the requested names. Their `WithName` metadata,
    // descriptions included, drops with them.
    let extra: Vec<Name> = export_registry
        .heads()
        .filter(|(name, _)| !requested.contains(name))
        .map(|(name, _)| name.clone())
        .collect();
    for name in extra {
        export_registry.remove_head(&name);
    }
    state::retain_kept(&mut export_registry);
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
/// Returns `true` when the path is absent, as on web, so that files without a
/// known path are accepted speculatively.
pub fn is_maybe_gantz(path: Option<&std::path::Path>) -> bool {
    path.map(is_gantz_path).unwrap_or(true)
}

/// Reserved registry name under which a copied subgraph travels inside a
/// clipboard `.gantz` document. See [`copied_to_string`].
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
    /// Registry dependencies referenced by copied nodes, such as Ref nodes,
    /// along with the heads and metadata naming them.
    pub registry: gantz_ca::Registry,
    /// The subgraph of selected nodes and their internal edges, in the
    /// stored data form.
    pub graph: DataGraph,
    /// Positions of nodes in the subgraph.
    pub positions: crate::Layout,
    /// The state of the copied nodes, by their paths in `graph`.
    pub state: state::Snapshot,
}

/// Build a [`Copied`] payload from the selected nodes in a graph. `state` is
/// the state of the nodes of the graph, by their paths in the graph.
///
/// The payload registry carries the transitive closure of the graphs the
/// selected nodes reference. It also carries the heads whose tips point at
/// those graphs, with their `WithName` and `WithCommit` metadata. Pasting
/// into another registry then restores names and views.
pub fn copy(
    registry: &gantz_ca::Registry,
    graph: &DataGraph,
    selected: &HashSet<node::graph::NodeIx>,
    layout: &crate::Layout,
    state: &[(Vec<node::Id>, gantz_ca::Datum)],
) -> Copied {
    let subgraph = gantz_core::graph::extract_subgraph(graph, selected);

    // Iterate selected nodes in sorted order alongside new node indices. This
    // matches `extract_subgraph`'s deterministic order.
    let sorted: std::collections::BTreeSet<_> = selected.iter().copied().collect();
    let pairs = sorted.iter().zip(subgraph.node_indices());
    let pairs = pairs.map(|(old, new)| (old.index(), new.index()));
    let positions = crate::view::remap(layout, pairs.clone()).collect();
    let state = state::remap_snapshot(state, pairs);

    // Collect registry deps transitively. Include the graphs the selected
    // nodes reference, and the graphs those graphs reference in turn, so the
    // whole subtree travels with the clipboard. Blob references ride along
    // likewise. The stored refs and blobs columns cover the whole walk with
    // no node lookups.
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

    // Include each collected graph's naming heads, so paste-merge restores
    // names and the text format's commits table still describes the named
    // graphs.
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
        state,
    }
}

/// Paste a [`Copied`] payload into a target graph.
///
/// Merges registry dependencies, adds the subgraph nodes and edges, and maps
/// positions with the given offset. Returns the new node indices in the
/// target graph.
pub fn paste(
    registry: &mut gantz_ca::Registry,
    target_graph: &mut DataGraph,
    target_layout: &mut crate::Layout,
    copied: &Copied,
    offset: emath::Vec2,
) -> Vec<node::graph::NodeIx> {
    registry.merge(copied.registry.clone());
    let new_indices = gantz_core::graph::add_subgraph(target_graph, &copied.graph);

    // Map positions from subgraph indices to target indices with offset.
    let pairs = copied.graph.node_indices().zip(&new_indices);
    let pairs = pairs.map(|(sub, target)| (sub.index(), target.index()));
    let positions = crate::view::remap(&copied.positions, pairs);
    target_layout.extend(positions.map(|(id, pos)| (id, pos + offset)));

    new_indices
}

/// Serialize a [`Copied`] payload as a `.gantz` document.
///
/// The copied subgraph rides as a graph named `clipboard` alongside the
/// registry dependencies. Its positions are stored as the clipboard commit's
/// view section entry. The whole payload is one ordinary `.gantz` document.
/// [`copied_from_str`] reverses this.
pub fn copied_to_string(
    copied: &Copied,
    codec: &DataCodec,
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
    if !copied.state.is_empty() {
        let stored =
            gantz_ca::section_insert::<state::Snapshots>(&mut registry, commit_ca, &copied.state);
        if let Err(e) = stored {
            log::warn!("the clipboard leaves out the state of the copied nodes: {e}");
        }
    }

    crate::format::to_string(&registry, codec)
}

/// Parse a clipboard payload produced by [`copied_to_string`].
///
/// Splits the `clipboard` graph and its positions back out from the registry
/// dependencies.
pub fn copied_from_str(text: &str, codec: &DataCodec) -> Result<Copied, ParseCopiedError> {
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
    let state = gantz_ca::section_get::<state::Snapshots>(&registry, &clip_ca).unwrap_or_default();

    // Everything reachable outside the clipboard commit is a dependency. The
    // export filters heads and views to the kept commits, so the `clipboard`
    // name and its view entry drop out with it.
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
        state,
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
    fn merge_keeps_existing_gui_sections() {
        let mut registry = test_registry();
        let ca = commit_addr_raw(10);
        let mut existing_view = crate::SceneView::default();
        existing_view
            .layout
            .insert(crate::NodeId(0), Default::default());
        crate::section::set_view(&mut registry, ca, &existing_view);
        crate::section::set_description(&mut registry, name("alpha"), "local".to_string());
        crate::section::set_demo(&mut registry, name("alpha"), "demo-a".to_string());

        let mut incoming = test_registry();
        crate::section::set_view(&mut incoming, ca, &crate::SceneView::default());
        crate::section::set_description(&mut incoming, name("alpha"), "imported".to_string());
        crate::section::set_demo(&mut incoming, name("alpha"), "demo-b".to_string());
        crate::section::set_description(&mut incoming, name("beta"), "new".to_string());
        registry.merge(incoming);

        // The existing view with one layout entry is preserved, not replaced.
        let view = crate::section::view(&registry, &ca).unwrap();
        assert_eq!(view.layout.len(), 1);
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
    /// and `WithName` metadata through the clipboard text round-trip.
    /// Positions ride the clipboard commit's view section entry.
    #[test]
    fn clipboard_round_trip_carries_positions_and_deps() {
        use crate::test_node::{TestGraph, codec, commit_named, expr, named_ref};

        let mut reg = gantz_ca::Registry::default();
        let mut leaf_g = TestGraph::default();
        leaf_g.add_node(expr("(+ 1 1)"));
        let (_, leaf_ga) = commit_named(&mut reg, Duration::from_secs(1), &leaf_g, &name("leaf"));
        crate::section::set_description(&mut reg, name("leaf"), "a leaf".to_string());

        // The working graph in data form. A ref to `leaf` plus a plain expr
        // node.
        let mut working = TestGraph::default();
        let a = working.add_node(named_ref("leaf", leaf_ga));
        let b = working.add_node(expr("(+ 2 2)"));
        let mut layout = crate::Layout::default();
        layout.insert(crate::NodeId::from_index(a.index()), emath::pos2(1.0, 2.0));
        layout.insert(crate::NodeId::from_index(b.index()), emath::pos2(3.0, 4.0));
        let selected: HashSet<_> = working.node_indices().collect();

        let copied = copy(&reg, &working, &selected, &layout, &[]);
        assert!(copied.registry.graph(&leaf_ga).is_some());
        assert!(copied.registry.head(&name("leaf")).is_some());

        let text = copied_to_string(&copied, &codec()).unwrap();
        let back: Copied = copied_from_str(&text, &codec()).unwrap();

        // The subgraph and its positions survive.
        assert_eq!(back.graph.node_count(), 2);
        for ix in [a, b] {
            let id = crate::NodeId::from_index(ix.index());
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
        let mut target_layout = crate::Layout::default();
        let new = paste(
            &mut target_reg,
            &mut target_graph,
            &mut target_layout,
            &back,
            emath::vec2(10.0, 10.0),
        );
        assert_eq!(new.len(), 2);
        assert!(target_reg.graph(&leaf_ga).is_some());
        assert!(target_reg.head(&name("leaf")).is_some());
        assert_eq!(
            target_layout.get(&crate::NodeId::from_index(new[0].index())),
            Some(&emath::pos2(11.0, 12.0)),
        );
    }

    /// The state of the copied nodes rides the clipboard by their paths in the
    /// subgraph. The state of the other nodes stays behind.
    #[test]
    fn copied_nodes_carry_their_state() {
        use crate::test_node::{TestGraph, codec, expr};
        use gantz_ca::Datum;

        let mut working = TestGraph::default();
        working.add_node(expr("(+ 1 1)"));
        let b = working.add_node(expr("(+ 2 2)"));
        let c = working.add_node(expr("(+ 3 3)"));
        let state = [
            (vec![0], Datum::I64(1)),
            (vec![1], Datum::I64(2)),
            (vec![2, 4], Datum::Sym("bd".to_string())),
        ];
        let selected = HashSet::from([b, c]);
        let reg = gantz_ca::Registry::default();
        let copied = copy(&reg, &working, &selected, &crate::Layout::default(), &state);
        let expected = vec![
            (vec![0], Datum::I64(2)),
            (vec![1, 4], Datum::Sym("bd".to_string())),
        ];
        assert_eq!(copied.state, expected);

        let text = copied_to_string(&copied, &codec()).unwrap();
        let back = copied_from_str(&text, &codec()).unwrap();
        assert_eq!(back.state, expected);
    }

    /// Exporting heads as text carries transitive deps and sections through
    /// a parse and merge into a fresh registry. Node state rides only with a
    /// name that keeps it.
    #[test]
    fn export_heads_text_round_trip() {
        use crate::test_node::{TestGraph, codec, commit_named, expr, named_ref};
        use gantz_core::node::state::{Keep, Snapshots};

        let mut reg = gantz_ca::Registry::default();
        let mut leaf_g = TestGraph::default();
        leaf_g.add_node(expr("(+ 1 1)"));
        let (leaf_ca, leaf_ga) =
            commit_named(&mut reg, Duration::from_secs(1), &leaf_g, &name("leaf"));
        let mut root_g = TestGraph::default();
        root_g.add_node(named_ref("leaf", leaf_ga));
        let (root_ca, root_ga) =
            commit_named(&mut reg, Duration::from_secs(2), &root_g, &name("root"));
        crate::section::set_description(&mut reg, name("root"), "the root".to_string());
        let mut view = crate::SceneView::default();
        view.layout.insert(crate::NodeId(0), emath::pos2(5.0, 6.0));
        crate::section::set_view(&mut reg, root_ca, &view);
        let state = vec![
            (vec![0], gantz_ca::Datum::Sym("bd".to_string())),
            (vec![1, 2], gantz_ca::Datum::ratio(1, 3).unwrap()),
            (vec![3], gantz_ca::Datum::F64(0.5)),
        ];
        gantz_ca::section_insert::<Snapshots>(&mut reg, root_ca, &state).unwrap();
        gantz_ca::section_insert::<Snapshots>(&mut reg, leaf_ca, &state).unwrap();
        gantz_ca::section_insert::<Keep>(&mut reg, name("root"), &true).unwrap();

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
            view.layout.get(&crate::NodeId(0)).copied(),
            Some(emath::pos2(5.0, 6.0)),
        );
        let restored = gantz_ca::section_get::<Snapshots>(&fresh, &root_ca);
        assert_eq!(restored, Some(state), "node state survives");
        assert!(gantz_ca::section_get::<Snapshots>(&fresh, &leaf_ca).is_none());
    }

    /// An import resolves a name that the file does not define to its head
    /// graph in the registry, and an old name to the graph of its new name.
    /// A name that the file defines shadows the registry.
    #[test]
    fn import_resolves_names_through_the_registry() {
        use crate::test_node::codec;

        let mut reg = gantz_ca::Registry::default();
        let lib = parse_export_at(b"(graph lib (x (expr 1)))", Duration::ZERO, &codec()).unwrap();
        reg.merge(lib);
        let lib_ga = reg.commits()[&reg.head(&name("lib")).unwrap()].graph;
        let renames = [(name("old"), name("lib"))];
        let import = |text: &str| {
            parse_import(
                text.as_bytes(),
                Duration::from_secs(1),
                &reg,
                &renames,
                &codec(),
            )
        };
        let user_refs = |import: &gantz_ca::Registry| -> Vec<(Name, GraphAddr)> {
            let ca = import.head(&name("user")).unwrap();
            let graph = import.commit_graph_ref(&ca).unwrap();
            crate::resync::named_refs(graph)
                .map(|(n, ga, _)| (n, ga))
                .collect()
        };

        let by_name = import("(graph user (r (ref lib)))").unwrap();
        assert_eq!(user_refs(&by_name), [(name("lib"), lib_ga)]);

        let by_old_name = import("(graph user (r (ref old)))").unwrap();
        assert_eq!(user_refs(&by_old_name), [(name("lib"), lib_ga)]);

        let own = import("(graph lib (x (expr 2)))\n(graph user (r (ref lib)))").unwrap();
        let own_ga = own.commits()[&own.head(&name("lib")).unwrap()].graph;
        assert_ne!(own_ga, lib_ga);
        assert_eq!(user_refs(&own), [(name("lib"), own_ga)]);

        let missing = import("(graph user (r (ref nowhere)))").unwrap_err();
        assert!(missing.is_missing_dependency(), "{missing}");
    }

    /// A base write-back carries the node state of a name that keeps it and
    /// leaves out the state of a name that does not.
    #[test]
    fn names_export_carries_only_kept_state() {
        use crate::test_node::{TestGraph, codec, commit_named, expr};
        use gantz_core::node::state::{Keep, Snapshots};

        let mut reg = gantz_ca::Registry::default();
        let mut g = TestGraph::default();
        g.add_node(expr("(+ 1 1)"));
        let (kept_ca, _) = commit_named(&mut reg, Duration::from_secs(1), &g, &name("kept"));
        g.add_node(expr("(+ 2 2)"));
        let (other_ca, _) = commit_named(&mut reg, Duration::from_secs(2), &g, &name("other"));
        let state = vec![(vec![0], gantz_ca::Datum::F64(0.5))];
        gantz_ca::section_insert::<Snapshots>(&mut reg, kept_ca, &state).unwrap();
        gantz_ca::section_insert::<Snapshots>(&mut reg, other_ca, &state).unwrap();
        gantz_ca::section_insert::<Keep>(&mut reg, name("kept"), &true).unwrap();

        let text = export_names_sexpr_named(&reg, ["kept", "other"], &codec()).unwrap();
        assert!(text.contains(&ContentAddr::from(kept_ca).to_string()));
        assert!(!text.contains(&ContentAddr::from(other_ca).to_string()));
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
