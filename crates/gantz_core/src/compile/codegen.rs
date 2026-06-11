//! Generation of the per-variant node fns (see [`node_fn`]) and the shared
//! fn-naming helpers.

use crate::node;
pub(crate) use node_fn::{name as node_fn_name, node_fns};

mod node_fn;

/// The string used to represent a path in a fn name.
pub(crate) fn path_string(path: &[node::Id]) -> String {
    path.iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(":")
}

/// Generate entry fn name from an `EntrypointId`.
///
/// The name is deterministic and unique - derived from the content hash
/// (truncated to 8 hex chars).
pub fn entry_fn_name(id: &super::EntrypointId) -> String {
    format!("entry-fn-{}", id.0.display_short())
}
