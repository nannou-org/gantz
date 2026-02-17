use gantz_ca::CaHash;
use gantz_core::node::{EvalConf, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_core::steel::SteelVal;
use serde::{Deserialize, Serialize};

/// A number stored in state. Can be updated via the first input.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash)]
#[cahash("gantz.number")]
pub struct Number;

impl gantz_core::Node for Number {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn push_eval(&self, _ctx: MetaCtx) -> Vec<EvalConf> {
        vec![EvalConf::All]
    }

    fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult {
        let expr = match ctx.inputs().get(0) {
            // If an input value was provided, use it to update state and
            // forward that value.
            Some(Some(val)) => {
                format!("(begin (if (number? {val}) (set! state {val}) void) state)")
            }
            // If no input value was provided, forward the value in state.
            _ => "(begin state)".to_string(),
        };
        gantz_core::node::parse_expr(&expr)
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        true
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        let path = ctx.path();
        gantz_core::node::state::init_value_if_absent(ctx.vm(), path, || SteelVal::NumV(0.0))
            .unwrap()
    }
}
