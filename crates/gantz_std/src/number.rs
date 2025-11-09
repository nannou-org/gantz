use gantz_core::{
    ca::CaHash,
    steel::{SteelVal, parser::ast::ExprKind, steel_vm::engine::Engine},
};
use serde::{Deserialize, Serialize};

/// A number stored in state. Can be updated via the first input.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct Number;

impl<Env> gantz_core::Node<Env> for Number {
    fn n_inputs(&self, _: &Env) -> usize {
        1
    }

    fn n_outputs(&self, _: &Env) -> usize {
        1
    }

    fn push_eval(&self, _: &Env) -> Vec<gantz_core::node::EvalConf> {
        vec![gantz_core::node::EvalConf::All]
    }

    fn expr(&self, ctx: gantz_core::node::ExprCtx<Env>) -> ExprKind {
        let expr = match ctx.inputs().get(0) {
            // If an input value was provided, use it to update state and
            // forward that value.
            Some(Some(val)) => {
                format!("(begin (if (number? {val}) (set! state {val}) void) state)")
            }
            // If no input value was provided, forward the value in state.
            _ => "(begin state)".to_string(),
        };
        Engine::emit_ast(&expr).unwrap().into_iter().next().unwrap()
    }

    fn stateful(&self) -> bool {
        true
    }

    fn register(&self, path: &[gantz_core::node::Id], vm: &mut Engine) {
        gantz_core::node::state::update_value(vm, path, SteelVal::NumV(0.0)).unwrap()
    }
}

impl CaHash for Number {
    fn hash(&self, hasher: &mut gantz_core::ca::blake3::Hasher) {
        "gantz_std::Number".hash(hasher);
    }
}
