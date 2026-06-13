//! Raises a registry into a [`Document`] and serializes it.
//!
//! The output mirrors the registry's three maps: a `(graph "<addr>" ...)` body
//! per graph, a flat `(commits ...)` table (one head commit per graph, for
//! validation), and a `(names ...)` table. Nodes get generated
//! `{keyword}{index}` labels and cross the typetag boundary as
//! `serde_json::Value`s. The returned [`Dumped`] also exposes, per graph, the
//! id and node labels emitted - everything an extender needs to attach its own
//! forms (e.g. `(layout ...)`) keyed by the same ids.

use crate::error::{ErrorKind, FormatError};
use crate::lower::Lowerable;
use crate::model::{
    Addr, CommitDecl, Conn, Document, Endpoint, GraphBody, GraphDef, NameDecl, NodeDecl, NodeSpec,
    RefSpec,
};
use crate::node_value::node_to_value;
use crate::sugar::keyword_for_tag;
use gantz_ca::{ContentAddr, GraphAddr, Registry};
use gantz_core::node::graph::Graph;
use petgraph::visit::{EdgeRef, IntoEdgeReferences};
use serde_json::Value;
use std::collections::HashMap;

/// The result of serializing a registry: the text plus the per-graph label
/// context an extender needs to emit its own forms.
pub struct Dumped {
    /// The serialized registry forms.
    pub text: String,
    /// Per graph address: the id emitted and the node index -> label map.
    pub graphs: HashMap<GraphAddr, GraphLabels>,
}

/// The id string and node labels emitted for a single graph.
pub struct GraphLabels {
    /// The file-local id used in the text (a short content address).
    pub id: String,
    /// Node index -> generated label.
    pub labels: HashMap<usize, String>,
}

/// Raise a registry into serialized text plus per-graph label context.
pub fn raise<N>(registry: &Registry<Graph<N>>) -> Result<Dumped, FormatError>
where
    N: Lowerable,
{
    let mut doc = Document::default();
    let mut graphs = HashMap::new();

    for (g_addr, graph) in registry.graphs() {
        let (body, labels) = graph_to_body::<N>(graph)?;
        let id = short_hex(*g_addr);
        doc.graphs.push(GraphDef {
            id: Addr::Concrete(id.clone()),
            body,
        });
        graphs.insert(*g_addr, GraphLabels { id, labels });
    }

    for (c_addr, commit) in registry.commits() {
        doc.commits.push(CommitDecl {
            id: Addr::Concrete(short_hex(*c_addr)),
            secs: commit.timestamp.as_secs(),
            nanos: commit.timestamp.subsec_nanos(),
            parent: commit.parent.map(|p| Addr::Concrete(short_hex(p))),
            graph: Addr::Concrete(short_hex(commit.graph)),
        });
    }

    for (name, c_addr) in registry.names() {
        doc.names.push(NameDecl {
            name: name.clone(),
            commit: Addr::Concrete(short_hex(*c_addr)),
        });
    }

    let text = crate::writer::write_document(&doc);
    Ok(Dumped { text, graphs })
}

// -- graph -> body -----------------------------------------------------------

/// Convert a graph into a [`GraphBody`], returning the index -> label map used
/// to resolve connections and (by extenders) layout positions.
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

// -- helpers -----------------------------------------------------------------

/// The first 8 hex characters of an address.
fn short_hex(addr: impl Into<ContentAddr>) -> String {
    let hex = addr.into().to_string();
    hex.get(..8).unwrap_or(&hex).to_string()
}
