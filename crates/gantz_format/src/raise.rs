//! Raises a registry into a [`Document`] and serializes it.
//!
//! The output mirrors the registry's three maps: a `(graph "<addr>" ...)` body
//! per graph, a flat `(commits ...)` table (one head commit per graph, for
//! validation), and a `(names ...)` table. Nodes get generated
//! `{keyword}{index}` labels and cross the node-type boundary as serde
//! [`Datum`]s. The returned [`Dumped`] also exposes, per graph, the id and node
//! labels emitted - everything an extender needs to attach its own forms (e.g.
//! `(layout ...)`) keyed by the same ids.

use crate::datum::{Datum, to_datum};
use crate::error::FormatError;
use crate::model::{
    Addr, CommitDecl, Conn, Document, Endpoint, GraphBody, GraphDef, NameDecl, NodeDecl, NodeSpec,
    RefSpec,
};
use crate::sugar::Sugar;
use gantz_ca::{ContentAddr, GraphAddr, Registry};
use gantz_core::node::graph::Graph;
use petgraph::visit::{EdgeRef, IntoEdgeReferences};
use serde::Serialize;
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
pub fn raise<N>(registry: &Registry<Graph<N>>, sugar: &dyn Sugar) -> Result<Dumped, FormatError>
where
    // Raising only ever *serializes* nodes, hence no `DeserializeOwned` bound.
    N: Serialize,
{
    let mut doc = Document::default();
    let mut graphs = HashMap::new();

    for (g_addr, graph) in registry.graphs() {
        let (body, labels) = graph_to_body::<N>(graph, sugar)?;
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

    let text = crate::writer::write_document(&doc, sugar);
    Ok(Dumped { text, graphs })
}

// -- graph -> body -----------------------------------------------------------

/// Convert a graph into a [`GraphBody`], returning the index -> label map used
/// to resolve connections and (by extenders) layout positions.
fn graph_to_body<N>(
    graph: &Graph<N>,
    sugar: &dyn Sugar,
) -> Result<(GraphBody, HashMap<usize, String>), FormatError>
where
    N: Serialize,
{
    let mut nodes = Vec::new();
    let mut labels: HashMap<usize, String> = HashMap::new();

    for ix in graph.node_indices() {
        let value =
            to_datum(&graph[ix]).map_err(|e| FormatError::node_deserialize("?", e.to_string()))?;
        let (spec, keyword) = node_spec_from_datum(value, sugar)?;
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

/// Convert a node's serde [`Datum`] into a [`NodeSpec`] and a label keyword.
fn node_spec_from_datum(
    value: Datum,
    sugar: &dyn Sugar,
) -> Result<(NodeSpec, String), FormatError> {
    let tag = value
        .get("type")
        .and_then(Datum::as_str)
        .unwrap_or("")
        .to_string();
    match tag.as_str() {
        "NamedRef" | "FnNamedRef" => {
            let func = tag == "FnNamedRef";
            let name = value
                .get("name")
                .and_then(Datum::as_str)
                .unwrap_or_default()
                .to_string();
            let hex = value
                .get("ref_")
                .and_then(Datum::as_str)
                .unwrap_or_default();
            let short = hex.get(..8).unwrap_or(hex).to_string();
            let sync = value.get("sync").and_then(Datum::as_bool).unwrap_or(false);
            let spec = NodeSpec::Ref(RefSpec {
                func,
                name,
                addr: Some(Addr::Concrete(short)),
                sync,
            });
            Ok((spec, if func { "fnref" } else { "ref" }.to_string()))
        }
        other => {
            let keyword = sugar.keyword_for_tag(other).unwrap_or("node").to_string();
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
