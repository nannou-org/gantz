//! Types for representing entrypoints into a gantz graph's generated code.

use crate::{
    Edge,
    node::{self, EvalConf, Node},
};
use gantz_ca::{self as ca, CaHash};
use petgraph::visit::{Data, IntoNodeReferences, NodeIndexable, NodeRef};
use std::{collections::BTreeSet, fmt};

/// Whether evaluation is pushed from or pulled to a node.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, CaHash)]
#[cahash("gantz.eval-kind")]
pub enum EvalKind {
    Push,
    Pull,
}

/// A single evaluation source within a graph tree.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, CaHash)]
#[cahash("gantz.eval-source")]
pub struct EvalSource {
    /// Full path to the node from root (e.g. `[5, 3]` = node 3 inside node 5).
    pub path: Vec<node::Id>,
    /// Whether this source pushes or pulls evaluation.
    pub kind: EvalKind,
    /// Which connections participate in evaluation.
    pub conf: EvalConf,
}

/// A set of eval sources to be evaluated together in one generated function.
///
/// Sources may span multiple graph nesting levels. During compilation,
/// sources are grouped by level and a FlowGraph is generated at each level.
/// The resulting eval fn concatenates all levels' statements, which is safe
/// because each level's statements access distinct parts of the state tree.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, CaHash)]
#[cahash("gantz.entrypoint")]
pub struct Entrypoint(pub BTreeSet<EvalSource>);

/// Canonical identifier for an entrypoint - the content-address hash.
///
/// Compact, deterministic, derived from the sorted source set.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EntrypointId(pub ca::ContentAddr);

impl fmt::Display for EntrypointId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl Entrypoint {
    /// Derive the canonical `EntrypointId` from this entrypoint's content hash.
    pub fn id(&self) -> EntrypointId {
        EntrypointId(ca::content_addr(self))
    }

    /// The parent path of the first source, if any.
    ///
    /// For root-level sources with `path.len() == 1`, the slice is empty.
    /// Returns `None` if the entrypoint has no sources.
    pub fn parent_path(&self) -> Option<&[node::Id]> {
        self.0
            .first()
            .map(|first| &first.path[..first.path.len() - 1])
    }
}

/// Create a singleton push entrypoint for the given node path with `EvalConf::All`.
///
/// Convenience for tests and callers that trigger a single push node.
pub fn push_entrypoint(path: Vec<node::Id>) -> Entrypoint {
    Entrypoint(BTreeSet::from([EvalSource {
        path,
        kind: EvalKind::Push,
        conf: EvalConf::All,
    }]))
}

/// Create a singleton pull entrypoint for the given node path with `EvalConf::All`.
///
/// Convenience for tests and callers that trigger a single pull node.
pub fn pull_entrypoint(path: Vec<node::Id>) -> Entrypoint {
    Entrypoint(BTreeSet::from([EvalSource {
        path,
        kind: EvalKind::Pull,
        conf: EvalConf::All,
    }]))
}

/// Default planner: one singleton entrypoint per push/pull eval node.
pub fn default_entrypoints<G>(get_node: node::GetNode<'_>, g: G) -> Vec<Entrypoint>
where
    G: Data<EdgeWeight = Edge> + IntoNodeReferences + NodeIndexable,
    G::NodeWeight: Node,
{
    let ctx = node::MetaCtx::new(get_node);
    let mut eps = Vec::new();
    for n_ref in g.node_references() {
        let id = g.to_index(n_ref.id());
        let node = n_ref.weight();
        for conf in node.push_eval(ctx) {
            eps.push(Entrypoint(BTreeSet::from([EvalSource {
                path: vec![id],
                kind: EvalKind::Push,
                conf,
            }])));
        }
        for conf in node.pull_eval(ctx) {
            eps.push(Entrypoint(BTreeSet::from([EvalSource {
                path: vec![id],
                kind: EvalKind::Pull,
                conf,
            }])));
        }
    }
    eps
}
