use gantz_core::steel::{parser::ast::ExprKind, steel_vm::engine::Engine};
use serde::{Deserialize, Serialize};

/// A simple node for pushing evaluation through the graph.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct Bang;

impl gantz_core::Node for Bang {
    fn n_outputs(&self) -> usize {
        1
    }

    fn expr(&self, _ctx: gantz_core::node::ExprCtx) -> ExprKind {
        Engine::emit_ast("'()").unwrap().into_iter().next().unwrap()
    }

    fn push_eval(&self) -> Vec<gantz_core::node::EvalConf> {
        vec![gantz_core::node::EvalConf::All]
    }
}
