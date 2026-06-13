//! Raises an [`Export`] into a [`File`] AST for serialization.
//!
//! The output mirrors the registry's three maps: a `(graph "<addr>" ...)` body
//! per graph, a flat `(commits ...)` table (one head commit per graph, for
//! validation), and a `(names ...)` table; plus `(layout ...)` keyed by graph
//! address and `(demo ...)` by name. Addresses are emitted as short hex; nodes
//! get generated `{keyword}{index}` labels and cross the typetag boundary as
//! `serde_json::Value`s.

use super::error::{ErrorKind, FormatError};
use super::lower::Lowerable;
use super::model::{
    Addr, CommitDecl, Conn, Demo, Endpoint, File, GraphBody, GraphDef, Layout, NameDecl, NodeDecl,
    NodeSpec, RefSpec,
};
use super::node_value::node_to_value;
use super::sugar::keyword_for_tag;
use crate::export::Export;
use gantz_ca::{ContentAddr, GraphAddr};
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

    // The top-level view of each graph, found via a commit that points at it.
    let mut graph_view: HashMap<GraphAddr, &egui_graph::View> = HashMap::new();
    for (commit_ca, gv) in &export.views {
        if let (Some(commit), Some(view)) = (reg.commits().get(commit_ca), gv.get(&Vec::new())) {
            graph_view.entry(commit.graph).or_insert(view);
        }
    }

    // One `(graph "<addr>" ...)` per graph body, with its layout.
    for (g_addr, graph) in reg.graphs() {
        let (body, labels) = graph_to_body::<N>(graph)?;
        file.graphs.push(GraphDef {
            id: short_addr(*g_addr),
            body,
        });
        if let Some(view) = graph_view.get(g_addr) {
            file.layouts.push(layout_for(*g_addr, view, &labels));
        }
    }

    // One commit per graph (the export's heads), for validation.
    for (c_addr, commit) in reg.commits() {
        file.commits.push(CommitDecl {
            id: short_addr(*c_addr),
            secs: commit.timestamp.as_secs(),
            nanos: commit.timestamp.subsec_nanos(),
            parent: commit.parent.map(short_addr),
            graph: short_addr(commit.graph),
        });
    }

    // Name -> commit mappings, and demos (keyed by name).
    for (name, c_addr) in reg.names() {
        file.names.push(NameDecl {
            name: name.clone(),
            commit: short_addr(*c_addr),
        });
        if let Some(demo) = export.demos.get(c_addr) {
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
            let short = hex.get(..8).unwrap_or(hex).to_string();
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

// -- layout ------------------------------------------------------------------

fn layout_for(
    g_addr: GraphAddr,
    view: &egui_graph::View,
    labels: &HashMap<usize, String>,
) -> Layout {
    let mut positions = Vec::new();
    for (node_id, pos) in &view.layout {
        if let Some(label) = labels.get(&(node_id.0 as usize)) {
            positions.push((label.clone(), pos.x, pos.y));
        }
    }
    // Stable order for deterministic output.
    positions.sort_by(|a, b| a.0.cmp(&b.0));
    let r = view.scene_rect;
    Layout {
        graph: short_addr(g_addr),
        positions,
        scene: Some([r.min.x, r.min.y, r.max.x, r.max.y]),
    }
}

// -- helpers -----------------------------------------------------------------

/// An address as a short-hex concrete [`Addr`] (the first 8 hex characters).
fn short_addr(addr: impl Into<ContentAddr>) -> Addr {
    let hex = addr.into().to_string();
    Addr::Concrete(hex.get(..8).unwrap_or(&hex).to_string())
}
