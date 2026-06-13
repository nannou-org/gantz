//! Raises an [`Export`] into a [`File`] AST for serialization.
//!
//! Each registry name becomes a `(graph ...)` (its head body), a
//! `(history ...)` (its commit chain, so commit identity round-trips), an
//! optional `(layout ...)` (its top-level view) and an optional `(demo ...)`.
//! Nodes are emitted in index order with generated `{keyword}{index}` labels;
//! node payloads cross the typetag boundary as `serde_json::Value`s.

use super::error::{ErrorKind, FormatError};
use super::lower::Lowerable;
use super::model::{
    Addr, CommitDecl, Conn, Demo, Endpoint, File, GraphBody, GraphDef, GraphId, History, Layout,
    NodeDecl, NodeSpec, RefSpec,
};
use super::node_value::node_to_value;
use super::sugar::keyword_for_tag;
use crate::export::Export;
use gantz_ca::{CommitAddr, ContentAddr};
use gantz_core::node::graph::Graph;
use petgraph::visit::{EdgeRef, IntoEdgeReferences};
use serde_json::Value;
use std::collections::HashMap;

/// Raise an [`Export`] into a [`File`] AST.
pub fn raise<N>(export: &Export<Graph<N>>) -> Result<File, FormatError>
where
    N: Lowerable,
{
    let reg = &export.registry;
    let mut file = File::default();

    for (name, &head_ca) in reg.names() {
        let commit = reg
            .commits()
            .get(&head_ca)
            .ok_or_else(|| FormatError::new(ErrorKind::MissingDependency(name.clone())))?;
        let head_graph_addr = commit.graph;
        let graph = reg
            .commit_graph_ref(&head_ca)
            .ok_or_else(|| FormatError::new(ErrorKind::MissingDependency(name.clone())))?;

        let (body, labels) = graph_to_body::<N>(graph)?;
        file.graphs.push(GraphDef {
            id: GraphId::Name(name.clone()),
            body,
        });

        file.histories
            .push(history_for(reg, name, head_ca, head_graph_addr));

        if let Some(layout) = layout_for(export, name, head_ca, &labels) {
            file.layouts.push(layout);
        }
        if let Some(demo) = export.demos.get(&head_ca) {
            file.demos.push(Demo {
                graph: name.clone(),
                demo: demo.clone(),
            });
        }
    }

    Ok(file)
}

// -- graph -> body -----------------------------------------------------------

/// Convert a graph into a [`GraphBody`], returning the index -> label map used
/// to resolve connections and layout positions.
fn graph_to_body<N>(graph: &Graph<N>) -> Result<(GraphBody, HashMap<usize, String>), FormatError>
where
    N: Lowerable,
{
    let mut nodes = Vec::new();
    let mut labels: HashMap<usize, String> = HashMap::new();

    for ix in graph.node_indices() {
        let value = node_to_value(&graph[ix]).map_err(|e| {
            FormatError::new(ErrorKind::NodeDeserialize {
                tag: "?".into(),
                msg: e.to_string(),
            })
        })?;
        let (spec, keyword) = node_spec_from_value::<N>(value)?;
        let label = format!("{keyword}{}", ix.index());
        labels.insert(ix.index(), label.clone());
        nodes.push(NodeDecl {
            name: label,
            index: None,
            spec,
        });
    }

    let mut conns = Vec::new();
    for edge in graph.edge_references() {
        let from = labels[&edge.source().index()].clone();
        let to = labels[&edge.target().index()].clone();
        let weight = edge.weight();
        conns.push(Conn {
            from: Endpoint {
                node: from,
                port: weight.output.0,
            },
            to: Endpoint {
                node: to,
                port: weight.input.0,
            },
        });
    }

    Ok((GraphBody { nodes, conns }, labels))
}

/// Convert a node's serde value into a [`NodeSpec`] and a label keyword.
fn node_spec_from_value<N>(value: Value) -> Result<(NodeSpec, &'static str), FormatError>
where
    N: Lowerable,
{
    let tag = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    match tag.as_str() {
        "NamedRef" | "FnNamedRef" => {
            let func = tag == "FnNamedRef";
            let name = value
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let hex = value
                .get("ref_")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let short = short_hex(hex);
            let sync = value.get("sync").and_then(Value::as_bool).unwrap_or(false);
            let spec = NodeSpec::Ref(RefSpec {
                func,
                name,
                addr: Some(Addr::Concrete(short)),
                sync,
            });
            Ok((spec, if func { "fnref" } else { "ref" }))
        }
        "GraphNode" => {
            let inner = value.get("graph").cloned().unwrap_or(Value::Null);
            let nested: Graph<N> = serde_json::from_value(inner).map_err(|e| {
                FormatError::new(ErrorKind::NodeDeserialize {
                    tag: "GraphNode".into(),
                    msg: e.to_string(),
                })
            })?;
            let (body, _) = graph_to_body::<N>(&nested)?;
            Ok((NodeSpec::Graph(body), "graph"))
        }
        other => {
            let keyword = keyword_for_tag(other).unwrap_or("node");
            Ok((NodeSpec::Value(value), keyword))
        }
    }
}

// -- history -----------------------------------------------------------------

fn history_for<N>(
    reg: &gantz_ca::Registry<Graph<N>>,
    name: &str,
    head_ca: CommitAddr,
    head_graph_addr: gantz_ca::GraphAddr,
) -> History {
    // Walk newest -> oldest along parent links that are present in the registry.
    let mut chain = Vec::new();
    let mut cur = Some(head_ca);
    while let Some(ca) = cur {
        match reg.commits().get(&ca) {
            Some(commit) => {
                chain.push((ca, commit.clone()));
                cur = commit.parent;
            }
            None => break,
        }
    }
    chain.reverse();

    let commits = chain
        .into_iter()
        .map(|(ca, commit)| CommitDecl {
            // Short id, verified by prefix on load.
            id: Addr::Concrete(short_hex(&ContentAddr::from(ca).to_string())),
            secs: commit.timestamp.as_secs(),
            nanos: commit.timestamp.subsec_nanos(),
            // Full parent hex so it parses even when the parent is pruned.
            parent: commit
                .parent
                .map(|p| Addr::Concrete(ContentAddr::from(p).to_string())),
            // Omit the graph for the head body, which is recomputed from the
            // named `(graph ...)` on load (healing any stale address with a
            // warning); ancestors spell theirs out, as their bodies are absent.
            graph: (commit.graph != head_graph_addr)
                .then(|| Addr::Concrete(ContentAddr::from(commit.graph).to_string())),
        })
        .collect();

    History {
        graph: name.to_string(),
        commits,
    }
}

// -- layout ------------------------------------------------------------------

fn layout_for<N>(
    export: &Export<Graph<N>>,
    name: &str,
    head_ca: CommitAddr,
    labels: &HashMap<usize, String>,
) -> Option<Layout> {
    let view = export.views.get(&head_ca)?.get(&Vec::new())?;
    let mut positions = Vec::new();
    for (node_id, pos) in &view.layout {
        if let Some(label) = labels.get(&(node_id.0 as usize)) {
            positions.push((label.clone(), pos.x, pos.y));
        }
    }
    // Stable order for deterministic output.
    positions.sort_by(|a, b| a.0.cmp(&b.0));
    let r = view.scene_rect;
    Some(Layout {
        graph: name.to_string(),
        path: Vec::new(),
        positions,
        scene: Some([r.min.x, r.min.y, r.max.x, r.max.y]),
    })
}

// -- helpers -----------------------------------------------------------------

/// The first 8 hex characters of an address string.
fn short_hex(hex: &str) -> String {
    hex.get(..8).unwrap_or(hex).to_string()
}
