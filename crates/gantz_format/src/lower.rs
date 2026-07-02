//! Lowers a [`Document`] into a registry, plus the context an extender needs.
//!
//! The document mirrors the registry's three maps: graph bodies (keyed by a
//! file-local id), a flat `(commits ...)` table and a `(names ...)` table.
//! Graphs are built in dependency order (a graph that `ref`s another is built
//! after it) so references resolve to already-known commits. A graph whose id
//! is a label and which no commit references is a hand-authored named graph: it
//! auto-registers under that label with a root commit synthesised at `now`.

use crate::datum::{Datum, from_datum};
use crate::error::{ErrorKind, FormatError};
use crate::model::{
    Addr, CommitDecl, Document, Form, GraphBody, GraphDef, NameDecl, NodeDecl, NodeSpec, RefSpec,
};
use gantz_ca::{CaHash, Commit, CommitAddr, ContentAddr, Registry, Timestamp};
use gantz_core::edge::Edge;
use gantz_core::node::graph::{Graph, NodeIx};
use gantz_core::node::{Input, Output};
use serde::de::DeserializeOwned;
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

/// The result of lowering a [`Document`]: the registry plus the resolution
/// context and preserved extra forms an extender needs.
pub struct Loaded<N> {
    /// The content-addressed registry.
    pub registry: Registry<Graph<N>>,
    /// graph id -> head commit.
    pub graph_head: HashMap<Addr, CommitAddr>,
    /// graph id -> node label -> node index.
    pub index: HashMap<Addr, HashMap<String, usize>>,
    /// registry name -> head commit.
    pub names: HashMap<String, CommitAddr>,
    /// Unrecognised top-level forms, preserved for an extender.
    pub extra: Vec<Form>,
}

/// Read-only reference-resolution context, threaded through graph building.
struct Resolve<'a> {
    /// name -> head commit, for already-built graphs.
    names: &'a HashMap<String, CommitAddr>,
    /// commit id -> commit, for resolving pinned references by id.
    commit_ids: &'a HashMap<Addr, CommitAddr>,
    /// every commit built so far, for resolving concrete-address prefixes.
    known: &'a [CommitAddr],
}

/// Lower a parsed [`Document`] into a [`Loaded`] registry, synthesising root
/// commits at `now` for any graph the `(commits ...)` table does not describe.
pub fn lower<N>(doc: Document, now: Timestamp) -> Result<Loaded<N>, FormatError>
where
    // `'static` is required by content-addressing (`Registry::add_graph`), whose
    // `for<'a> &'a Graph<N>` bound only holds for all `'a` when `N: 'static`.
    // Lowering only ever *deserializes* nodes, hence no `Serialize` bound.
    N: DeserializeOwned + CaHash + 'static,
{
    let Document {
        graphs,
        commits,
        names: name_decls,
        descriptions,
        extra,
    } = doc;

    // Index the document's three tables.
    let graphs_by_id: HashMap<Addr, &GraphDef> = graphs.iter().map(|g| (g.id.clone(), g)).collect();
    // graph id -> the commit pointing at it (at most one per graph).
    let commit_for_graph: HashMap<Addr, &CommitDecl> =
        commits.iter().map(|c| (c.graph.clone(), c)).collect();
    // commit id -> graph id.
    let graph_of_commit: HashMap<Addr, Addr> = commits
        .iter()
        .map(|c| (c.id.clone(), c.graph.clone()))
        .collect();
    // commit id -> names pointing at it.
    let mut names_of_commit: HashMap<Addr, Vec<String>> = HashMap::new();
    for decl in &name_decls {
        names_of_commit
            .entry(decl.commit.clone())
            .or_default()
            .push(decl.name.clone());
    }

    // name -> graph id, used to order graphs by their references.
    let name_to_graph_id =
        compute_name_to_graph_id(&graphs, &name_decls, &commit_for_graph, &graph_of_commit);
    let order = topo_order(&graphs, &graphs_by_id, &name_to_graph_id)?;

    let mut registry: Registry<Graph<N>> =
        Registry::new(HashMap::new(), HashMap::new(), BTreeMap::new());
    let mut names: HashMap<String, CommitAddr> = HashMap::new();
    let mut commit_ids: HashMap<Addr, CommitAddr> = HashMap::new();
    let mut known: Vec<CommitAddr> = Vec::new();
    let mut graph_head: HashMap<Addr, CommitAddr> = HashMap::new();
    let mut index: HashMap<Addr, HashMap<String, usize>> = HashMap::new();

    for id in &order {
        let def = graphs_by_id[id];
        let resolve = Resolve {
            names: &names,
            commit_ids: &commit_ids,
            known: &known,
        };
        let (graph, index_map) = build_graph::<N>(&def.body, &resolve)?;
        let g_addr = registry.add_graph(graph);
        index.insert(id.clone(), index_map);

        // Build the head commit: from the table where present, else a fresh root.
        let head = match commit_for_graph.get(id) {
            Some(decl) => build_commit(&mut registry, decl, g_addr, &commit_ids, &mut known),
            None => {
                let ca = registry.add_commit(Commit::new(now, None, g_addr));
                known.push(ca);
                ca
            }
        };
        graph_head.insert(id.clone(), head);

        // Register names for this commit: explicit ones from the names table,
        // plus an auto-name for an un-committed label graph (hand-authored).
        let mut register = |name: String| {
            names.insert(name.clone(), head);
            registry.insert_name(name, head);
        };
        match commit_for_graph.get(id) {
            Some(decl) => {
                commit_ids.insert(decl.id.clone(), head);
                if let Some(ns) = names_of_commit.get(&decl.id) {
                    ns.iter().for_each(|n| register(n.clone()));
                }
            }
            None => {
                if let Addr::Label(label) = id {
                    register(label.clone());
                }
            }
        }
    }

    // Apply name-keyed descriptions (independent of how each name registered).
    for decl in descriptions {
        registry.set_description(decl.name, decl.description);
    }

    Ok(Loaded {
        registry,
        graph_head,
        index,
        names,
        extra,
    })
}

// -- graph construction ------------------------------------------------------

fn build_graph<N>(
    body: &GraphBody,
    resolve: &Resolve,
) -> Result<(Graph<N>, HashMap<String, usize>), FormatError>
where
    N: DeserializeOwned,
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
    N: DeserializeOwned,
{
    match &decl.spec {
        NodeSpec::Value(v) => node_from_datum::<N>(v.clone()),
        NodeSpec::Ref(refspec) => {
            let v = resolve_ref_value(refspec, resolve)?;
            node_from_datum::<N>(v)
        }
    }
}

fn node_from_datum<N>(datum: Datum) -> Result<N, FormatError>
where
    N: DeserializeOwned,
{
    let tag = datum
        .get("type")
        .and_then(Datum::as_str)
        .unwrap_or("?")
        .to_string();
    from_datum::<N>(datum).map_err(|e| FormatError::node_deserialize(tag, e.to_string()))
}

fn resolve_ref_value(refspec: &RefSpec, resolve: &Resolve) -> Result<Datum, FormatError> {
    let commit_ca =
        match &refspec.addr {
            None => resolve.names.get(&refspec.name).copied().ok_or_else(|| {
                FormatError::new(ErrorKind::MissingDependency(refspec.name.clone()))
            })?,
            Some(Addr::Label(label)) => resolve
                .commit_ids
                .get(&Addr::Label(label.clone()))
                .copied()
                .ok_or_else(|| FormatError::new(ErrorKind::MissingDependency(label.clone())))?,
            // A pinned address is advisory: if it no longer resolves (e.g. the
            // commit healed because the format keeps only the head commit per
            // graph, so a parent was dropped and the address recomputed), fall
            // back to the reference's name. This keeps `NamedRef`s - including
            // nested-graph refs in a copy/paste payload - resolving across the
            // address drift (#232).
            Some(Addr::Concrete(hex)) => resolve_commit(hex, resolve.known)
                .or_else(|| resolve.names.get(&refspec.name).copied())
                .ok_or_else(|| FormatError::new(ErrorKind::MissingDependency(hex.clone())))?,
        };
    let content: ContentAddr = commit_ca.into();
    let hex = content.to_string();
    let tag = if refspec.func {
        "FnNamedRef"
    } else {
        "NamedRef"
    };
    Ok(Datum::tagged(
        tag,
        vec![
            ("ref_".to_string(), Datum::Str(hex)),
            ("name".to_string(), Datum::Str(refspec.name.clone())),
            ("sync".to_string(), Datum::Bool(refspec.sync)),
        ],
    ))
}

// -- commits -----------------------------------------------------------------

/// Build the head commit described by `decl`, pointing at `g_addr` (the graph
/// it references, which has just been built).
fn build_commit<N>(
    registry: &mut Registry<Graph<N>>,
    decl: &CommitDecl,
    g_addr: gantz_ca::GraphAddr,
    commit_ids: &HashMap<Addr, CommitAddr>,
    known: &mut Vec<CommitAddr>,
) -> CommitAddr {
    let parent = resolve_parent(&decl.parent, commit_ids, known);
    let timestamp = Duration::new(decl.secs, decl.nanos);
    let mut commit = Commit::new(timestamp, parent, g_addr);
    // Merge parents resolve like the first parent; an absent one is dropped
    // (`resolve_parent` re-roots, which for an *extra* parent means dropping).
    commit.merge_parents = decl
        .merge_parents
        .iter()
        .filter_map(|addr| resolve_parent(&Some(addr.clone()), commit_ids, known))
        .collect();
    let commit_ca = registry.add_commit(commit);
    // A declared id may not match the recomputed address - e.g. the format
    // keeps only the head commit per graph, so a dropped parent re-roots the
    // commit and changes its hash. This is routine (refs recover by name in
    // `resolve_ref_value`), so it is logged at debug rather than warned.
    if let Addr::Concrete(hex) = &decl.id {
        let computed = ContentAddr::from(commit_ca).to_string();
        if !computed.starts_with(hex.as_str()) {
            log::debug!(
                "commit `{hex}` no longer matches its contents (recomputed `{computed}`); \
                 using the recomputed address",
            );
        }
    }
    known.push(commit_ca);
    commit_ca
}

/// Resolve a commit's declared parent to a present commit. A parent absent from
/// the document re-roots the commit; this is routine (the format keeps only the
/// head commit per graph, so history parents are commonly absent), so it is
/// logged at debug rather than warned.
fn resolve_parent(
    parent: &Option<Addr>,
    commit_ids: &HashMap<Addr, CommitAddr>,
    known: &[CommitAddr],
) -> Option<CommitAddr> {
    match parent {
        None => None,
        Some(addr @ Addr::Label(label)) => match commit_ids.get(addr) {
            Some(ca) => Some(*ca),
            None => {
                log::debug!("commit parent label `{label}` not present; recorded as a root commit");
                None
            }
        },
        Some(Addr::Concrete(hex)) => match resolve_commit(hex, known) {
            Some(ca) => Some(ca),
            None => {
                log::debug!("commit parent `{hex}` not present; recorded as a root commit");
                None
            }
        },
    }
}

// -- dependency ordering -----------------------------------------------------

/// Map each registry name to the graph id it ultimately points at, via the
/// names + commits tables, plus auto-names for un-committed label graphs.
fn compute_name_to_graph_id(
    graphs: &[GraphDef],
    name_decls: &[NameDecl],
    commit_for_graph: &HashMap<Addr, &CommitDecl>,
    graph_of_commit: &HashMap<Addr, Addr>,
) -> HashMap<String, Addr> {
    let mut out = HashMap::new();
    for decl in name_decls {
        if let Some(graph_id) = graph_of_commit.get(&decl.commit) {
            out.insert(decl.name.clone(), graph_id.clone());
        }
    }
    for def in graphs {
        if let Addr::Label(label) = &def.id {
            if !commit_for_graph.contains_key(&def.id) {
                out.entry(label.clone()).or_insert_with(|| def.id.clone());
            }
        }
    }
    out
}

/// Topologically order graph ids so that a graph is built after every graph it
/// references (by name). Returns an error on a reference cycle.
fn topo_order(
    graphs: &[GraphDef],
    graphs_by_id: &HashMap<Addr, &GraphDef>,
    name_to_graph_id: &HashMap<String, Addr>,
) -> Result<Vec<Addr>, FormatError> {
    let mut order = Vec::new();
    let mut state: HashMap<Addr, u8> = HashMap::new(); // 0 visiting, 1 done
    for def in graphs {
        visit(
            &def.id,
            graphs_by_id,
            name_to_graph_id,
            &mut state,
            &mut order,
        )?;
    }
    Ok(order)
}

fn visit(
    id: &Addr,
    graphs_by_id: &HashMap<Addr, &GraphDef>,
    name_to_graph_id: &HashMap<String, Addr>,
    state: &mut HashMap<Addr, u8>,
    order: &mut Vec<Addr>,
) -> Result<(), FormatError> {
    match state.get(id) {
        Some(1) => return Ok(()),
        Some(0) => {
            return Err(FormatError::new(ErrorKind::CycleInRefs(vec![format!(
                "{id:?}"
            )])));
        }
        _ => {}
    }
    state.insert(id.clone(), 0);
    if let Some(def) = graphs_by_id.get(id) {
        for name in referenced_names(&def.body) {
            if let Some(dep) = name_to_graph_id.get(&name) {
                if graphs_by_id.contains_key(dep) {
                    visit(dep, graphs_by_id, name_to_graph_id, state, order)?;
                }
            }
        }
    }
    state.insert(id.clone(), 1);
    order.push(id.clone());
    Ok(())
}

/// All names referenced by `ref`/`fn-ref` within a graph body (recursively).
fn referenced_names(body: &GraphBody) -> Vec<String> {
    let mut names = Vec::new();
    for decl in &body.nodes {
        match &decl.spec {
            NodeSpec::Ref(r) => names.push(r.name.clone()),
            NodeSpec::Value(_) => {}
        }
    }
    names
}

// -- address helpers ---------------------------------------------------------

/// Resolve a concrete address (full hex or unambiguous prefix) to a *present*
/// commit. A prefix is ambiguous only when it matches two distinct commits.
fn resolve_commit(hex: &str, known: &[CommitAddr]) -> Option<CommitAddr> {
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
