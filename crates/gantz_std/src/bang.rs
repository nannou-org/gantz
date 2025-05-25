use gantz_core::steel::{parser::ast::ExprKind, steel_vm::engine::Engine};
use serde::{Deserialize, Serialize};

/// A simple node for pushing evaluation through the graph.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct Bang;

impl gantz_core::Node for Bang {
    fn n_outputs(&self) -> usize {
        1
    }

    fn expr(&self, _inputs: &[Option<ExprKind>]) -> ExprKind {
        Engine::emit_ast("'()").unwrap().into_iter().next().unwrap()
    }

    fn push_eval(&self) -> Option<gantz_core::node::EvalFn> {
        Some(gantz_core::node::EvalFn)
    }
}
