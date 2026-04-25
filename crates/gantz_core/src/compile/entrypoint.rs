//! Types for representing entrypoints into a gantz graph's generated code.

use crate::{
    Edge,
    node::{self, Node},
    visit,
};
use gantz_ca::{self as ca, CaHash};
use petgraph::visit::{Data, IntoEdgesDirected, IntoNodeReferences, NodeIndexable, Visitable};
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
    /// Which connections participate in evaluation (resolved, not deferred).
    pub conns: node::Conns,
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
}

/// Create an `EvalSource` at the given path with all `n_conns` connections active.
pub fn source(path: Vec<node::Id>, kind: EvalKind, n_conns: u8) -> EvalSource {
    debug_assert!(!path.is_empty(), "EvalSource path must be non-empty");
    EvalSource {
        path,
        kind,
        // u8 is always within Conns::MAX (256).
        conns: node::Conns::connected(n_conns as usize).unwrap(),
    }
}

/// Create a push `EvalSource` at the given path with all `n_outputs` connections active.
pub fn push_source(path: Vec<node::Id>, n_outputs: u8) -> EvalSource {
    source(path, EvalKind::Push, n_outputs)
}

/// Create a pull `EvalSource` at the given path with all `n_inputs` connections active.
pub fn pull_source(path: Vec<node::Id>, n_inputs: u8) -> EvalSource {
    source(path, EvalKind::Pull, n_inputs)
}

/// Create an entrypoint from a single evaluation source.
pub fn from_source(source: EvalSource) -> Entrypoint {
    Entrypoint(BTreeSet::from([source]))
}

/// Create an entrypoint from multiple evaluation sources.
pub fn from_sources(sources: impl IntoIterator<Item = EvalSource>) -> Entrypoint {
    Entrypoint(sources.into_iter().collect())
}

/// Create a singleton push entrypoint for the given node path with all
/// `n_outputs` connections active.
///
/// Convenience for tests and callers that trigger a single push node.
pub fn push(path: Vec<node::Id>, n_outputs: u8) -> Entrypoint {
    from_source(push_source(path, n_outputs))
}

/// Create a singleton pull entrypoint for the given node path with all
/// `n_inputs` connections active.
///
/// Convenience for tests and callers that trigger a single pull node.
pub fn pull(path: Vec<node::Id>, n_inputs: u8) -> Entrypoint {
    from_source(pull_source(path, n_inputs))
}

/// Visitor that collects entrypoints from all nodes in a graph tree.
struct EntrypointCollector<'a> {
    get_node: node::GetNode<'a>,
    eps: Vec<Entrypoint>,
}

impl visit::Visitor for EntrypointCollector<'_> {
    fn visit_pre(&mut self, ctx: visit::Ctx<'_, '_>, node: &dyn Node) {
        let meta_ctx = node::MetaCtx::new(self.get_node);
        let path = ctx.path().to_vec();
        let n_outputs = node.n_outputs(meta_ctx);
        for conf in node.push_eval(meta_ctx) {
            let conns = super::meta::conns_from_eval_conf(&conf, n_outputs)
                .expect("push_eval conf exceeds output count");
            self.eps.push(from_source(EvalSource {
                path: path.clone(),
                kind: EvalKind::Push,
                conns,
            }));
        }
        let n_inputs = node.n_inputs(meta_ctx);
        for conf in node.pull_eval(meta_ctx) {
            let conns = super::meta::conns_from_eval_conf(&conf, n_inputs)
                .expect("pull_eval conf exceeds input count");
            self.eps.push(from_source(EvalSource {
                path: path.clone(),
                kind: EvalKind::Pull,
                conns,
            }));
        }
    }
}

/// Collect one singleton entrypoint per push/pull eval node.
///
/// Uses the visitor pattern to recurse into nested and referenced graphs,
/// discovering entrypoints at all nesting levels.
pub fn push_pull_entrypoints<G>(get_node: node::GetNode<'_>, g: G) -> Vec<Entrypoint>
where
    G: Data<EdgeWeight = Edge> + IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable,
    G::NodeWeight: Node,
{
    let mut collector = EntrypointCollector {
        get_node,
        eps: vec![],
    };
    crate::graph::visit(get_node, g, &[], &mut collector);
    collector.eps
}
