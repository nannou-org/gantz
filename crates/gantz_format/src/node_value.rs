//! The generic bridge between node specs and typetag nodes.
//!
//! Every node type round-trips through a self-describing [`serde_json::Value`]:
//! deserialization dispatches on the `"type"` tag via `typetag`, so each node's
//! own `Deserialize`/`Serialize` runs unchanged and new node types need no
//! bespoke support. This is the single seam where the format meets the concrete
//! node set; it is validated by the gate test in the `gantz` crate
//! (`typetag_roundtrips_through_serde_json_value`).

use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

/// Deserialize a node from its serde object representation
/// (`{ "type": <tag>, .. }`).
pub fn value_to_node<N: DeserializeOwned>(value: Value) -> Result<N, serde_json::Error> {
    serde_json::from_value(value)
}

/// Serialize a node to its serde object representation.
pub fn node_to_value<N: Serialize>(node: &N) -> Result<Value, serde_json::Error> {
    serde_json::to_value(node)
}
