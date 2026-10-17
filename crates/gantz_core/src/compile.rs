//! Items related to generating steel code from a gantz graph, primarily the
//! [`module`] fn.
//!
//! ## Pipeline
//!
//! 1. Meta, in `meta`. A visitor walk collects one [`Meta`] per graph level
//!    into a rose tree. A `Meta` holds adjacency, arities, branch masks and
//!    the stateful, inlet, outlet and delay sets.
//! 2. Lowering, in `lower`. Each level lowers once per use. Graph fns lower
//!    once per active-inlet variant. Level bodies lower once per entrypoint.
//!    Each body's node order comes from `order`. The result is an IR, in `ir`, of node-call steps, branch dispatches
//!    and join points. A join point is a local fn whose parameters carry
//!    reconverging values. Values consumed outside a branch construct ride
//!    its exports. `ir::validate` checks the IR's scoping and arity
//!    invariants on every lowering. See [`Config::validate_ir`]. A violation
//!    is a compiler bug and surfaces as [`ModuleError::InvalidIr`].
//! 3. Outlet-activation analysis, in `analysis`. The distinct sets of a
//!    level's outlets that can fire together. Abstract interpretation of the
//!    lowered IR computes them, forking per branch arm, so the patterns
//!    cannot drift from the emitted code. Backs `Graph::branches` and
//!    push-through-outlet propagation.
//! 4. Emission, in `emit`. A mechanical walk from IR to Steel restricted to
//!    primitive base-engine forms, plus the per-variant node fns in
//!    `codegen`. `link` assembles the emitted fns into the module.
//!    Definition order is semantic because Steel resolves free identifiers
//!    at definition. Deepest levels come first. A level's node fns precede
//!    its graph fns. Entry fns come last.
//!
//! Nested graphs compile call-based. Each active-inlet variant becomes one
//! `graph-fn-{path}-i{mask}`, called like a node fn. An entrypoint sourced
//! inside a nested graph becomes a per-level fn. The fn threads the level's
//! state and returns the outlet result. A stateful level fn returns
//! `(list result state)`. When the push reaches the outlets through
//! branching, the result is a `(list branch-ix vals)` pair. The parent level
//! continues from it as a pre-bound source.
//!
//! Cycles are legal when they pass through a [`crate::node::Delay`].
//! Ordering and reachability never propagate through a delay. Its value
//! crosses between evaluations. It is read at the top of a level body and
//! written where its input is produced.

pub(crate) use analysis::level_branch_patterns;
#[doc(inline)]
pub use entrypoint::{
    Entrypoint, EntrypointId, EvalKind, EvalSource, pull_source, push_pull_entrypoints, push_source,
};
#[doc(inline)]
pub use error::ModuleError;
pub use link::module;
pub(crate) use lower::{NodeConf, NodeConns};
use meta::MetaTree;
#[doc(inline)]
pub use meta::{EdgeKind, Meta, MetaGraph};
pub(crate) use names::graph_fn_name;
#[doc(inline)]
pub use names::{Name, entry_fn_name};
pub(crate) use order::eval_order;
pub use order::{Edges, pull_eval_order};
pub(crate) use rosetree::RoseTree;
#[doc(inline)]
pub use source_map::SourceMap;

mod analysis;
mod codegen;
mod emit;
pub mod entrypoint;
pub mod error;
mod ir;
mod link;
mod lower;
mod meta;
pub mod names;
mod order;
mod rosetree;
pub mod source_map;

/// Options controlling [`module`] compilation.
///
/// The defaults suit regular builds. [`validate_ir`][Self::validate_ir] is
/// an optimisation toggle. [`emit_all_node_fns`][Self::emit_all_node_fns]
/// is a codegen debugging toggle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Config {
    /// Check the scoping and arity invariants of every lowered IR body. A
    /// violation surfaces as [`ModuleError::InvalidIr`]. It is a bug in the
    /// compiler itself, never in the compiled graph.
    ///
    /// On by default. Disable it as an optimisation. An internal compiler
    /// error then surfaces further downstream, for example as a confusing
    /// Steel evaluation error, instead of at lowering with a precise
    /// diagnosis.
    pub validate_ir: bool,
    /// Emit the all-connected variant node fn for every node, not only the
    /// variants called by some lowered evaluation.
    ///
    /// Off by default. Node fns are normally emitted on demand, so a node
    /// that no entrypoint's evaluation calls produces no code. Enable this to
    /// inspect any node's generated code before anything calls it. The extra
    /// definitions are never called and do not affect evaluation.
    pub emit_all_node_fns: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            validate_ir: true,
            emit_all_node_fns: false,
        }
    }
}
