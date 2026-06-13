//! Lowers a [`File`] AST into an [`Export`].
//!
//! Graphs are built in dependency order (a graph that `ref`s another is built
//! after it), so each reference resolves to an already-known commit address.
//! Node indices follow declaration order. Commit identity comes from an
//! explicit `(history ...)` block when present (reproducing every
//! [`CommitAddr`] exactly), or is synthesised as a single root commit at `now`
//! for hand-authored files with no history.

use super::error::{ErrorKind, FormatError};
use super::model::{
    Addr, File, GraphBody, GraphDef, GraphId, History, Layout, NodeDecl, NodeSpec, RefSpec,
};
use super::node_value::value_to_node;
use crate::GraphViews;
use crate::export::Export;
use gantz_ca::{Commit, CommitAddr, ContentAddr, GraphAddr, Registry, Timestamp};
use gantz_core::edge::Edge;
use gantz_core::node::graph::{Graph, GraphNode, NodeIx};
use gantz_core::node::{Input, Output};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

/// The bounds a node type must satisfy to be lowered from the text format.
///
/// `'static` is required because content-addressing borrows the graph for
/// every lifetime (`for<'a> &'a Graph<N>: ..`).
pub trait Lowerable:
    'static + serde::Serialize + serde::de::DeserializeOwned + gantz_ca::CaHash
{
}
impl<N> Lowerable for N where
    N: 'static + serde::Serialize + serde::de::DeserializeOwned + gantz_ca::CaHash
{
}

/// Read-only reference-resolution context, threaded through graph building.
struct Resolve<'a> {
    /// name -> head commit, for already-built graphs.
    heads: &'a HashMap<String, CommitAddr>,
    /// label -> commit, for history placeholders of already-built graphs.
    labels: &'a HashMap<String, CommitAddr>,
    /// every commit built so far, for resolving concrete-address prefixes.
    known: &'a [CommitAddr],
}

/// Lower a parsed [`File`] into an [`Export`], synthesising commits at `now`
/// where no history is provided.
pub fn lower<N>(file: File, now: Timestamp) -> Result<Export<Graph<N>>, FormatError>
where
    N: Lowerable,
{
    // Index named graphs and histories.
    let mut defs: BTreeMap<String, &GraphDef> = BTreeMap::new();
    for def in &file.graphs {
        if let GraphId::Name(name) = &def.id {
            defs.insert(name.clone(), def);
        }
    }
    let histories: HashMap<&str, &History> = file
        .histories
        .iter()
        .map(|h| (h.graph.as_str(), h))
        .collect();

    let order = topo_order(&defs)?;

    let mut registry: Registry<Graph<N>> =
        Registry::new(HashMap::new(), HashMap::new(), BTreeMap::new());
    let mut heads: HashMap<String, CommitAddr> = HashMap::new();
    let mut labels: HashMap<String, CommitAddr> = HashMap::new();
    let mut known: Vec<CommitAddr> = Vec::new();
    // name -> (head commit, node-name -> index) for layout resolution.
    let mut index_maps: HashMap<String, HashMap<String, usize>> = HashMap::new();

    for name in &order {
        let def = defs[name];
        let resolve = Resolve {
            heads: &heads,
            labels: &labels,
            known: &known,
        };
        let (graph, index_map) = build_graph::<N>(&def.body, &resolve)?;
        // The registry computes and returns the body's address.
        let g_addr = registry.add_graph(graph);

        let head = match histories.get(name.as_str()) {
            Some(history) => {
                // `build_history` records every commit (including the head) in
                // `known`.
                build_history(
                    &mut registry,
                    history,
                    name,
                    g_addr,
                    &mut labels,
                    &mut known,
                )?
            }
            None => {
                let ca = registry.add_commit(Commit::new(now, None, g_addr));
                registry.insert_name(name.clone(), ca);
                known.push(ca);
                ca
            }
        };

        heads.insert(name.clone(), head);
        index_maps.insert(name.clone(), index_map);
    }

    let views = build_views(&file.layouts, &heads, &index_maps)?;
    let demos = build_demos(&file.demos, &heads);

    Ok(Export {
        registry,
        views,
        demos,
    })
}

// -- graph construction ------------------------------------------------------

fn build_graph<N>(
    body: &GraphBody,
    resolve: &Resolve,
) -> Result<(Graph<N>, HashMap<String, usize>), FormatError>
where
    N: Lowerable,
{
    let mut graph: Graph<N> = Graph::default();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut node_ix: HashMap<String, NodeIx> = HashMap::new();

    for decl in &body.nodes {
        if index.contains_key(&decl.name) {
            return Err(FormatError::new(ErrorKind::DuplicateNode(
                decl.name.clone(),
            )));
        }
        let node = build_node::<N>(decl, resolve)?;
        let ix = graph.add_node(node);
        index.insert(decl.name.clone(), ix.index());
        node_ix.insert(decl.name.clone(), ix);
    }

    for conn in &body.conns {
        let from = *node_ix
            .get(&conn.from.node)
            .ok_or_else(|| FormatError::new(ErrorKind::UnknownNode(conn.from.node.clone())))?;
        let to = *node_ix
            .get(&conn.to.node)
            .ok_or_else(|| FormatError::new(ErrorKind::UnknownNode(conn.to.node.clone())))?;
        graph.add_edge(
            from,
            to,
            Edge::new(Output(conn.from.port), Input(conn.to.port)),
        );
    }

    Ok((graph, index))
}

fn build_node<N>(decl: &NodeDecl, resolve: &Resolve) -> Result<N, FormatError>
where
    N: Lowerable,
{
    match &decl.spec {
        NodeSpec::Value(v) => node_from_value::<N>(v.clone()),
        NodeSpec::Ref(refspec) => {
            let v = resolve_ref_value(refspec, resolve)?;
            node_from_value::<N>(v)
        }
        NodeSpec::Graph(body) => {
            let (nested, _) = build_graph::<N>(body, resolve)?;
            let gn = GraphNode { graph: nested };
            let mut v = serde_json::to_value(&gn).map_err(|e| {
                FormatError::new(ErrorKind::NodeDeserialize {
                    tag: "GraphNode".into(),
                    msg: e.to_string(),
                })
            })?;
            if let Value::Object(ref mut map) = v {
                map.insert("type".into(), Value::String("GraphNode".into()));
            }
            node_from_value::<N>(v)
        }
    }
}

fn node_from_value<N>(v: Value) -> Result<N, FormatError>
where
    N: serde::de::DeserializeOwned,
{
    let tag = v
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_string();
    value_to_node::<N>(v).map_err(|e| {
        FormatError::new(ErrorKind::NodeDeserialize {
            tag,
            msg: e.to_string(),
        })
    })
}

fn resolve_ref_value(refspec: &RefSpec, resolve: &Resolve) -> Result<Value, FormatError> {
    let commit_ca =
        match &refspec.addr {
            None => resolve.heads.get(&refspec.name).copied().ok_or_else(|| {
                FormatError::new(ErrorKind::MissingDependency(refspec.name.clone()))
            })?,
            Some(Addr::Label(label)) => resolve
                .labels
                .get(label)
                .copied()
                .ok_or_else(|| FormatError::new(ErrorKind::MissingDependency(label.clone())))?,
            Some(Addr::Concrete(hex)) => resolve_commit(hex, resolve.known)
                .ok_or_else(|| FormatError::new(ErrorKind::MissingDependency(hex.clone())))?,
        };
    let content: ContentAddr = commit_ca.into();
    let hex = content.to_string();
    let tag = if refspec.func {
        "FnNamedRef"
    } else {
        "NamedRef"
    };
    Ok(json!({
        "type": tag,
        "ref_": hex,
        "name": refspec.name,
        "sync": refspec.sync,
    }))
}

// -- commit history ----------------------------------------------------------

fn build_history<N>(
    registry: &mut Registry<Graph<N>>,
    history: &History,
    name: &str,
    head_graph_addr: GraphAddr,
    labels: &mut HashMap<String, CommitAddr>,
    known: &mut Vec<CommitAddr>,
) -> Result<CommitAddr, FormatError> {
    let mut head: Option<CommitAddr> = None;
    for decl in &history.commits {
        // An omitted graph means the named head body; ancestors carry their own
        // (possibly body-less) graph address.
        let graph_addr = match &decl.graph {
            None => head_graph_addr,
            Some(Addr::Concrete(hex)) => GraphAddr::from(parse_full_addr(hex)?),
            Some(Addr::Label(l)) => {
                return Err(FormatError::new(ErrorKind::BadAddr(format!(
                    "graph label `{l}` unsupported"
                ))));
            }
        };
        // The parent is taken from the declaration (not chain order) so a
        // dangling/pruned parent still reproduces the exact commit address.
        let parent = match &decl.parent {
            None => None,
            Some(Addr::Concrete(hex)) => Some(CommitAddr::from(parse_full_addr(hex)?)),
            Some(Addr::Label(l)) => Some(
                *labels
                    .get(l)
                    .ok_or_else(|| FormatError::new(ErrorKind::MissingDependency(l.clone())))?,
            ),
        };
        let timestamp = Duration::new(decl.secs, decl.nanos);
        // The registry computes the commit address from its contents.
        let commit_ca = registry.add_commit(Commit::new(timestamp, parent, graph_addr));
        // `add_commit` clears a parent that is not present in the registry. Look
        // the commit back up to detect that and warn - the file referenced an
        // ancestor we do not have, so this commit became a root.
        if let Some(declared_parent) = parent {
            let kept = registry
                .commits()
                .get(&commit_ca)
                .and_then(|c| c.parent)
                .is_some();
            if !kept {
                log::warn!(
                    "commit for `{name}` referenced absent parent `{}`; recorded it as a root commit",
                    ContentAddr::from(declared_parent),
                );
            }
        }
        // A declared id that no longer matches the recomputed address means the
        // file is stale (e.g. the hashing changed). Warn and heal rather than
        // fail; references pinned to the old address may not resolve (a planned
        // follow-up is to remap such references).
        if let Addr::Concrete(hex) = &decl.id {
            let computed = ContentAddr::from(commit_ca).to_string();
            if !computed.starts_with(hex.as_str()) {
                log::warn!(
                    "commit `{hex}` for `{name}` no longer matches its contents \
                     (recomputed `{computed}`); using the recomputed address",
                );
            }
        }
        if let Addr::Label(label) = &decl.id {
            labels.insert(label.clone(), commit_ca);
        }
        known.push(commit_ca);
        head = Some(commit_ca);
    }
    let head = head.ok_or_else(|| {
        FormatError::new(ErrorKind::Malformed(format!(
            "history for `{name}` is empty"
        )))
    })?;
    registry.insert_name(name.to_string(), head);
    Ok(head)
}

// -- views / demos -----------------------------------------------------------

fn build_views(
    layouts: &[Layout],
    heads: &HashMap<String, CommitAddr>,
    index_maps: &HashMap<String, HashMap<String, usize>>,
) -> Result<HashMap<CommitAddr, GraphViews>, FormatError> {
    let mut views: HashMap<CommitAddr, GraphViews> = HashMap::new();
    for layout in layouts {
        // Only top-level layouts (no descent path) are supported for now.
        if !layout.path.is_empty() {
            continue;
        }
        let Some(&head) = heads.get(&layout.graph) else {
            continue;
        };
        let Some(index_map) = index_maps.get(&layout.graph) else {
            continue;
        };
        let mut egui_layout = egui_graph::Layout::default();
        for (name, x, y) in &layout.positions {
            if let Some(&ix) = index_map.get(name) {
                egui_layout.insert(egui_graph::NodeId(ix as u64), egui::Pos2::new(*x, *y));
            }
        }
        let scene_rect = match layout.scene {
            Some([min_x, min_y, max_x, max_y]) => {
                egui::Rect::from_min_max(egui::pos2(min_x, min_y), egui::pos2(max_x, max_y))
            }
            None => egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(0.0, 0.0)),
        };
        let view = egui_graph::View {
            scene_rect,
            layout: egui_layout,
        };
        views.entry(head).or_default().insert(Vec::new(), view);
    }
    Ok(views)
}

fn build_demos(
    demos: &[super::model::Demo],
    heads: &HashMap<String, CommitAddr>,
) -> HashMap<CommitAddr, String> {
    let mut out = HashMap::new();
    for demo in demos {
        if let Some(&head) = heads.get(&demo.graph) {
            out.insert(head, demo.demo.clone());
        }
    }
    out
}

// -- dependency ordering -----------------------------------------------------

/// Topologically order names so that a graph is built after every name it
/// references. Returns an error on a reference cycle.
fn topo_order(defs: &BTreeMap<String, &GraphDef>) -> Result<Vec<String>, FormatError> {
    let mut order = Vec::new();
    let mut state: HashMap<String, u8> = HashMap::new(); // 0 visiting, 1 done
    for name in defs.keys() {
        visit(name, defs, &mut state, &mut order)?;
    }
    Ok(order)
}

fn visit(
    name: &str,
    defs: &BTreeMap<String, &GraphDef>,
    state: &mut HashMap<String, u8>,
    order: &mut Vec<String>,
) -> Result<(), FormatError> {
    match state.get(name) {
        Some(1) => return Ok(()),
        Some(0) => {
            return Err(FormatError::new(ErrorKind::CycleInRefs(vec![
                name.to_string(),
            ])));
        }
        _ => {}
    }
    state.insert(name.to_string(), 0);
    if let Some(def) = defs.get(name) {
        for dep in referenced_names(&def.body) {
            if defs.contains_key(&dep) {
                visit(&dep, defs, state, order)?;
            }
        }
    }
    state.insert(name.to_string(), 1);
    order.push(name.to_string());
    Ok(())
}

/// All names referenced by `ref`/`fn-ref` within a graph body (recursively).
fn referenced_names(body: &GraphBody) -> Vec<String> {
    let mut names = Vec::new();
    for decl in &body.nodes {
        match &decl.spec {
            NodeSpec::Ref(r) => names.push(r.name.clone()),
            NodeSpec::Graph(nested) => names.extend(referenced_names(nested)),
            NodeSpec::Value(_) => {}
        }
    }
    names
}

// -- address helpers ---------------------------------------------------------

fn parse_full_addr(hex: &str) -> Result<ContentAddr, FormatError> {
    hex.parse::<ContentAddr>()
        .map_err(|_| FormatError::new(ErrorKind::BadAddr(hex.to_string())))
}

/// Resolve a concrete address (full hex or unambiguous prefix) to a commit.
///
/// A prefix is ambiguous only when it matches two *distinct* commits;
/// duplicate entries for the same commit are fine.
fn resolve_commit(hex: &str, known: &[CommitAddr]) -> Option<CommitAddr> {
    if hex.len() == 64 {
        if let Ok(content) = hex.parse::<ContentAddr>() {
            return Some(CommitAddr::from(content));
        }
    }
    let mut matches: Vec<CommitAddr> = known
        .iter()
        .copied()
        .filter(|ca| ContentAddr::from(*ca).to_string().starts_with(hex))
        .collect();
    matches.sort();
    matches.dedup();
    match matches.as_slice() {
        [only] => Some(*only),
        _ => None,
    }
}
