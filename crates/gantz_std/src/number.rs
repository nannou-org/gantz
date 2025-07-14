use gantz_core::steel::{SteelVal, parser::ast::ExprKind, steel_vm::engine::Engine};
use serde::{Deserialize, Serialize};

/// A number stored in state. Can be updated via the first input.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct Number;

impl gantz_core::Node for Number {
    fn n_inputs(&self) -> usize {
        1
    }

    fn n_outputs(&self) -> usize {
        1
    }

    fn push_eval(&self) -> Option<gantz_core::node::EvalFn> {
        Some(gantz_core::node::EvalFn)
    }

    fn expr(&self, ctx: gantz_core::node::ExprCtx) -> ExprKind {
        let expr = match ctx.inputs().get(0) {
            // If an input value was provided, use it to update state and
            // forward that value.
            Some(Some(val)) => {
                format!("(begin (when (number? {val}) (set! state {val})) state)")
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
