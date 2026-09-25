//! Lowers a [`Document`] into a registry, plus the context an extender needs.
//!
//! The document mirrors the registry's three maps. Those are graph bodies
//! keyed by a file-local id, a flat `(commits ...)` table and a `(names ...)`
//! table. Graphs are built in dependency order, so a graph that `ref`s another
//! is built after it and references resolve to known commits. A graph whose id
//! is a label and which no commit references is a hand-authored named graph.
//! It auto-registers under that label with a root commit synthesised at `now`.

use crate::datum::{Datum, from_datum};
use crate::error::{ErrorKind, FormatError};
use crate::model::{
    Addr, CommitDecl, Document, Form, GraphBody, GraphDef, NameDecl, NodeDecl, NodeSpec, RefSpec,
    SectionKey,
};
use gantz_ca::{
    Commit, CommitAddr, ContentAddr, DataGraph, GraphAddr, Key, NodeData, Registry, Timestamp,
};
use gantz_core::edge::Edge;
use gantz_core::node::{Input, Output};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

/// The node-normalization seam threaded through [`crate::from_str_normalized`].
/// It turns one parsed node's `"type"`-tagged [`Datum`] into its canonical
/// stored [`NodeData`], with fields validated and the refs and blobs columns
/// recomputed.
///
/// The node-set entry points such as [`crate::from_str`] supply the node set's
/// serde round-trip. Richer layers such as a GUI's value-level codec supply
/// their own.
pub type Normalize<'a> = dyn Fn(Datum) -> Result<NodeData, FormatError> + 'a;

/// The result of lowering a [`Document`]. It holds the registry plus the
/// resolution context and preserved extra forms an extender needs.
///
/// The registry stores graphs in their erased data form. See
/// [`gantz_core::data`]. The node-set type the document was lowered through is
/// only the codec, not part of the result.
pub struct Loaded {
    /// The content-addressed registry.
    pub registry: Registry,
    /// The head commit of each graph id.
    pub graph_head: HashMap<Addr, CommitAddr>,
    /// The node index of each node label, per graph id.
    pub index: HashMap<Addr, HashMap<String, usize>>,
    /// The head commit of each registry name.
    pub names: HashMap<String, CommitAddr>,
    /// Unrecognised top-level forms, preserved for an extender.
    pub extra: Vec<Form>,
}

/// Read-only reference-resolution context, threaded through graph building.
///
/// References resolve to graph addresses, which are content identities. A
/// by-name reference resolves to the graph at the name's head. A pinned
/// address is a graph address.
struct Resolve<'a> {
    /// The head graph of each name, for already-built graphs.
    name_graphs: &'a HashMap<String, GraphAddr>,
    /// The graph of each commit id, for resolving pinned references by commit
    /// label.
    commit_graphs: &'a HashMap<Addr, GraphAddr>,
    /// Every graph built so far, for resolving concrete-address prefixes.
    known: &'a [GraphAddr],
    /// The head graph of each externally-known name, consulted as a fallback
    /// after the document's own names. See [`crate::from_str_seeded`].
    seed: &'a BTreeMap<String, GraphAddr>,
}

/// Lower a parsed [`Document`] into a [`Loaded`] registry, synthesising root
/// commits at `now` for named label graphs with no explicit commit.
pub fn lower<N>(doc: Document, now: Timestamp) -> Result<Loaded, FormatError>
where
    N: Serialize + DeserializeOwned + gantz_core::Node,
{
    lower_seeded::<N>(doc, now, &BTreeMap::new())
}

/// As [`lower`], but names the document does not define resolve through
/// `seed`. See [`crate::from_str_seeded`] for how the seed affects content
/// addresses.
pub fn lower_seeded<N>(
    doc: Document,
    now: Timestamp,
    seed: &BTreeMap<String, GraphAddr>,
) -> Result<Loaded, FormatError>
where
    // Nodes are deserialized through the node set's serde, then erased back
    // to data for storage. `gantz_core::data::erase_node` requires
    // `Serialize` and `Node`. Registry addresses are always computed on the
    // erased form.
    N: Serialize + DeserializeOwned + gantz_core::Node,
{
    lower_normalized(doc, now, seed, &serde_normalize::<N>)
}

/// As [`lower_seeded`], with an explicit [`Normalize`] seam in place of a
/// node-set type parameter.
pub fn lower_normalized(
    doc: Document,
    now: Timestamp,
    seed: &BTreeMap<String, GraphAddr>,
    normalize: &Normalize,
) -> Result<Loaded, FormatError> {
    let Document {
        graphs,
        mut commits,
        names: mut name_decls,
        sections,
        extra,
    } = doc;

    // Index the document's three tables.
    let mut graphs_by_id = HashMap::new();
    for graph in &graphs {
        if graphs_by_id.insert(graph.id.clone(), graph).is_some() {
            return Err(FormatError::malformed(format!(
                "duplicate graph declaration {:?}",
                graph.id
            )));
        }
    }
    let mut commit_declarations = HashMap::new();
    for commit in &commits {
        if commit_declarations.insert(commit.id.clone(), ()).is_some() {
            return Err(FormatError::malformed(format!(
                "duplicate commit declaration {:?}",
                commit.id
            )));
        }
    }
    // Resolve prefixes against the complete declaration set before ordering.
    // Looking only at already-built content makes ancestry and references
    // depend on document order, and can hide an ambiguous prefix.
    for commit in &mut commits {
        for parent in commit.parent.iter_mut().chain(&mut commit.merge_parents) {
            if let Some(id) = declared_id(parent, &commit_declarations)? {
                *parent = id.clone();
            }
        }
        if let Some(id) = declared_id(&commit.graph, &graphs_by_id)? {
            commit.graph = id.clone();
        }
    }
    let mut declared_names = std::collections::HashSet::new();
    for name in &mut name_decls {
        if !declared_names.insert(name.name.clone()) {
            return Err(FormatError::malformed(format!(
                "duplicate name declaration {:?}",
                name.name
            )));
        }
        if let Some(id) = declared_id(&name.commit, &commit_declarations)? {
            name.commit = id.clone();
        }
    }
    // The last declared commit for each graph supplies the friendly layout head.
    // Every declared commit is retained independently below.
    let commit_for_graph: HashMap<Addr, &CommitDecl> =
        commits.iter().map(|c| (c.graph.clone(), c)).collect();
    // The graph id of each commit id.
    let graph_of_commit: HashMap<Addr, Addr> = commits
        .iter()
        .map(|c| (c.id.clone(), c.graph.clone()))
        .collect();

    // The graph id of each name, used to order graphs by their references.
    let name_to_graph_id =
        compute_name_to_graph_id(&graphs, &name_decls, &commit_for_graph, &graph_of_commit);
    let order = topo_order(&graphs, &graphs_by_id, &name_to_graph_id, &graph_of_commit)?;

    let mut registry: Registry = Registry::default();
    let mut names: HashMap<String, CommitAddr> = HashMap::new();
    let mut name_graphs: HashMap<String, GraphAddr> = HashMap::new();
    let mut commit_ids: HashMap<Addr, CommitAddr> = HashMap::new();
    let mut commit_graphs: HashMap<Addr, GraphAddr> = HashMap::new();
    let mut known_commits: Vec<CommitAddr> = Vec::new();
    let mut known_graphs: Vec<GraphAddr> = Vec::new();
    let mut graph_head: HashMap<Addr, CommitAddr> = HashMap::new();
    let mut index: HashMap<Addr, HashMap<String, usize>> = HashMap::new();
    let mut graph_ids = HashMap::new();

    for id in &order {
        let def = graphs_by_id[id];
        let resolve = Resolve {
            name_graphs: &name_graphs,
            commit_graphs: &commit_graphs,
            known: &known_graphs,
            seed,
        };
        let (data_graph, index_map) = build_graph(&def.body, &resolve, normalize)?;
        let g_addr = registry.add_graph(data_graph);
        known_graphs.push(g_addr);
        index.insert(id.clone(), index_map);
        graph_ids.insert(id.clone(), g_addr);
        // References need graph identities, not commit identities. Register
        // these before lowering history, whose parent order is independent.
        for (name, graph_id) in &name_to_graph_id {
            if graph_id == id {
                name_graphs.insert(name.clone(), g_addr);
            }
        }
        for decl in commits.iter().filter(|decl| &decl.graph == id) {
            commit_graphs.insert(decl.id.clone(), g_addr);
        }
        if let Addr::Label(label) = id
            && !commit_for_graph.contains_key(id)
        {
            let head = registry.add_commit(Commit::new(now, None, g_addr));
            known_commits.push(head);
            graph_head.insert(id.clone(), head);
            names.insert(label.clone(), head);
            registry.set_head(label.parse().expect("infallible"), head);
        }
    }

    // Lower every commit only after its declared parents. Equal graph content,
    // equal timestamps and merge parents must not collapse historical identity.
    let declarations: HashMap<_, _> = commits.iter().map(|decl| (decl.id.clone(), decl)).collect();
    let mut pending: Vec<_> = commits.iter().collect();
    while !pending.is_empty() {
        let before = pending.len();
        let mut deferred = Vec::new();
        for decl in pending {
            if decl
                .parent
                .iter()
                .chain(&decl.merge_parents)
                .any(|parent| declarations.contains_key(parent) && !commit_ids.contains_key(parent))
            {
                deferred.push(decl);
                continue;
            }
            let graph = graph_ids.get(&decl.graph).copied().ok_or_else(|| {
                FormatError::new(ErrorKind::MissingDependency(format!("{:?}", decl.graph)))
            })?;
            let head = build_commit(&mut registry, decl, graph, &commit_ids, &mut known_commits);
            commit_ids.insert(decl.id.clone(), head);
        }
        if deferred.len() == before {
            return Err(FormatError::malformed("commit ancestry contains a cycle"));
        }
        pending = deferred;
    }
    for (id, decl) in &commit_for_graph {
        graph_head.insert(id.clone(), commit_ids[&decl.id]);
    }
    for decl in name_decls {
        let head = commit_ids.get(&decl.commit).copied().ok_or_else(|| {
            FormatError::new(ErrorKind::MissingDependency(format!("{:?}", decl.commit)))
        })?;
        names.insert(decl.name.clone(), head);
        registry.set_head(decl.name.parse().expect("infallible"), head);
    }

    // Apply generic metadata sections, semantics as declared in the text.
    for section in sections {
        for (key, value) in section.entries {
            let Some(key) = lower_section_key(key) else {
                continue;
            };
            registry.set_section_value(
                section.id.clone(),
                section.policy,
                section.liveness,
                key,
                gantz_ca::Value::Datum(value),
            );
        }
    }

    Ok(Loaded {
        registry,
        graph_head,
        index,
        names,
        extra,
    })
}

/// Resolve labels exactly and address prefixes uniquely across all declarations.
fn declared_id<'a, T>(
    address: &Addr,
    declarations: &'a HashMap<Addr, T>,
) -> Result<Option<&'a Addr>, FormatError> {
    let Addr::Concrete(prefix) = address else {
        return Ok(declarations.get_key_value(address).map(|(id, _)| id));
    };
    let mut matches = declarations.keys().filter(|id| {
        matches!(id, Addr::Concrete(declared) if declared.starts_with(prefix) || prefix.starts_with(declared))
    });
    let first = matches.next();
    if matches.next().is_some() {
        return Err(FormatError::new(ErrorKind::BadAddr(format!(
            "ambiguous address prefix {prefix:?}"
        ))));
    }
    Ok(first)
}

/// Convert a text-form section key to a registry key. Malformed hex keys are
/// dropped. Advisory metadata degrades and never fails a load.
fn lower_section_key(key: SectionKey) -> Option<Key> {
    match key {
        SectionKey::Name(name) => Some(Key::Name(name.parse().expect("infallible"))),
        SectionKey::Commit(hex) => Some(Key::Commit(hex.parse::<ContentAddr>().ok()?.into())),
        SectionKey::Graph(hex) => Some(Key::Graph(hex.parse::<ContentAddr>().ok()?.into())),
        SectionKey::Addr(hex) => Some(Key::Addr(hex.parse().ok()?)),
    }
}

fn build_graph(
    body: &GraphBody,
    resolve: &Resolve,
    normalize: &Normalize,
) -> Result<(DataGraph, HashMap<String, usize>), FormatError> {
    let mut graph = DataGraph::default();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut node_ix = HashMap::new();

    for decl in &body.nodes {
        if index.contains_key(&decl.name) {
            return Err(FormatError::new(ErrorKind::DuplicateNode(
                decl.name.clone(),
            )));
        }
        let node = build_node(decl, resolve, normalize)?;
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

fn build_node(
    decl: &NodeDecl,
    resolve: &Resolve,
    normalize: &Normalize,
) -> Result<NodeData, FormatError> {
    match &decl.spec {
        NodeSpec::Value(v) => normalize(v.clone()),
        NodeSpec::Ref(refspec) => {
            let v = resolve_ref_value(refspec, resolve)?;
            normalize(v)
        }
    }
}

/// The node-set-serde [`Normalize`] implementation backing [`lower`] and
/// [`lower_seeded`]. It deserializes the tagged datum through `N`, then erases
/// the node back to its canonical data form.
fn serde_normalize<N>(datum: Datum) -> Result<NodeData, FormatError>
where
    N: Serialize + DeserializeOwned + gantz_core::Node,
{
    let tag = datum
        .get("type")
        .and_then(Datum::as_str)
        .unwrap_or("?")
        .to_string();
    let node: N =
        from_datum(datum).map_err(|e| FormatError::node_deserialize(tag.clone(), e.to_string()))?;
    gantz_core::data::erase_node(&node)
        .map_err(|e| FormatError::node_deserialize(tag, e.to_string()))
}

fn resolve_ref_value(refspec: &RefSpec, resolve: &Resolve) -> Result<Datum, FormatError> {
    let graph_ca = match &refspec.addr {
        None => resolve
            .name_graphs
            .get(&refspec.name)
            .or_else(|| resolve.seed.get(&refspec.name))
            .copied()
            .ok_or_else(|| FormatError::new(ErrorKind::MissingDependency(refspec.name.clone())))?,
        Some(Addr::Label(label)) => resolve
            .commit_graphs
            .get(&Addr::Label(label.clone()))
            .copied()
            .ok_or_else(|| FormatError::new(ErrorKind::MissingDependency(label.clone())))?,
        // A pinned address is advisory. If it does not resolve, fall back to
        // the reference's name. This is rare, since a graph address is
        // independent of commit re-rooting, but a document may pin a graph
        // version it does not carry.
        Some(Addr::Concrete(hex)) => resolve_graph(hex, resolve.known)
            .or_else(|| resolve.name_graphs.get(&refspec.name).copied())
            .or_else(|| resolve.seed.get(&refspec.name).copied())
            .ok_or_else(|| FormatError::new(ErrorKind::MissingDependency(hex.clone())))?,
    };
    let content: ContentAddr = graph_ca.into();
    let hex = content.to_string();
    let tag = if refspec.func {
        "FnNamedRef"
    } else {
        "NamedRef"
    };
    // Mirror `gantz_core::node::Ref`'s serde shape. A bare address when the
    // reference carries no extension data, else a map of address and ext.
    let ref_value = match &refspec.ext {
        None => Datum::Str(hex),
        Some(ext) => Datum::Map(vec![
            ("addr".to_string(), Datum::Str(hex)),
            ("ext".to_string(), ext.clone()),
        ]),
    };
    Ok(Datum::tagged(
        tag,
        vec![
            ("ref_".to_string(), ref_value),
            ("name".to_string(), Datum::Str(refspec.name.clone())),
            ("sync".to_string(), Datum::Bool(refspec.sync)),
        ],
    ))
}

/// Build the head commit described by `decl`, pointing at the just-built graph
/// `g_addr`.
fn build_commit(
    registry: &mut Registry,
    decl: &CommitDecl,
    g_addr: gantz_ca::GraphAddr,
    commit_ids: &HashMap<Addr, CommitAddr>,
    known: &mut Vec<CommitAddr>,
) -> CommitAddr {
    let parent = resolve_parent(&decl.parent, commit_ids, known);
    let timestamp = Duration::new(decl.secs, decl.nanos);
    let mut commit = Commit::new(timestamp, parent, g_addr);
    // Merge parents resolve like the first parent. `resolve_parent` re-roots
    // an absent parent, which for an extra parent means dropping it.
    commit.merge_parents = decl
        .merge_parents
        .iter()
        .filter_map(|addr| resolve_parent(&Some(addr.clone()), commit_ids, known))
        .collect();
    let commit_ca = registry.add_commit(commit);
    // A declared id may not match the recomputed address when a document omits
    // a parent or normalization changes a node's canonical representation.
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
/// the document re-roots the commit and is logged at debug rather than warned.
fn resolve_parent(
    parent: &Option<Addr>,
    commit_ids: &HashMap<Addr, CommitAddr>,
    known: &[CommitAddr],
) -> Option<CommitAddr> {
    if let Some(ca) = parent.as_ref().and_then(|addr| commit_ids.get(addr)) {
        return Some(*ca);
    }
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

/// Map each registry name to the graph id it points at via the names and
/// commits tables. Includes auto-names for label graphs with no commit.
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
/// references by name. Returns an error on a reference cycle.
fn topo_order(
    graphs: &[GraphDef],
    graphs_by_id: &HashMap<Addr, &GraphDef>,
    name_to_graph_id: &HashMap<String, Addr>,
    graph_of_commit: &HashMap<Addr, Addr>,
) -> Result<Vec<Addr>, FormatError> {
    let mut order = Vec::new();
    let mut state: HashMap<Addr, u8> = HashMap::new(); // 0 visiting, 1 done
    for def in graphs {
        visit(
            &def.id,
            graphs_by_id,
            name_to_graph_id,
            graph_of_commit,
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
    graph_of_commit: &HashMap<Addr, Addr>,
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
        for node in &def.body.nodes {
            let NodeSpec::Ref(reference) = &node.spec else {
                continue;
            };
            let pinned = match &reference.addr {
                Some(addr @ Addr::Label(_)) => graph_of_commit.get(addr),
                Some(addr @ Addr::Concrete(_)) => declared_id(addr, graphs_by_id)?,
                None => None,
            };
            if let Some(dep) = pinned.or_else(|| name_to_graph_id.get(&reference.name)) {
                if graphs_by_id.contains_key(dep) {
                    visit(
                        dep,
                        graphs_by_id,
                        name_to_graph_id,
                        graph_of_commit,
                        state,
                        order,
                    )?;
                }
            }
        }
    }
    state.insert(id.clone(), 1);
    order.push(id.clone());
    Ok(())
}

/// Resolve a concrete address to a present commit. The address is full hex or
/// an unambiguous prefix. A prefix is ambiguous only when it matches two
/// distinct commits.
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

/// Resolve a concrete address to a present graph. The address is full hex or
/// an unambiguous prefix. A prefix is ambiguous only when it matches two
/// distinct graphs.
fn resolve_graph(hex: &str, known: &[GraphAddr]) -> Option<GraphAddr> {
    let mut matches: Vec<GraphAddr> = known
        .iter()
        .copied()
        .filter(|ga| ContentAddr::from(*ga).to_string().starts_with(hex))
        .collect();
    matches.sort();
    matches.dedup();
    match matches.as_slice() {
        [only] => Some(*only),
        _ => None,
    }
}
