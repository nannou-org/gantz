use gantz_core::steel::{parser::ast::ExprKind, steel_vm::engine::Engine};
use serde::{Deserialize, Serialize};

/// A simple node for pushing evaluation through the graph.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct Bang;

impl<Env> gantz_core::Node<Env> for Bang {
    fn n_outputs(&self, _: &Env) -> usize {
        1
    }

    fn expr(&self, _ctx: gantz_core::node::ExprCtx<Env>) -> ExprKind {
        Engine::emit_ast("'()").unwrap().into_iter().next().unwrap()
    }

    fn push_eval(&self, _: &Env) -> Vec<gantz_core::node::EvalConf> {
        vec![gantz_core::node::EvalConf::All]
    }
}

impl gantz_ca::CaHash for Bang {
    fn hash(&self, hasher: &mut gantz_ca::Hasher) {
        "gantz_std::Bang".hash(hasher);
    }
}
