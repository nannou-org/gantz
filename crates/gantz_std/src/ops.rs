use gantz_ca::CaHash;
use gantz_core::node::{ExprCtx, ExprResult, MetaCtx};
use serde::{Deserialize, Serialize};

/// Simple `Add` operation node.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash)]
#[cahash("gantz.add")]
pub struct Add;

impl gantz_core::Node for Add {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        2
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult {
        let inputs = ctx.inputs();
        let (l, r) = match (inputs.get(0), inputs.get(1)) {
            (Some(Some(l)), Some(Some(r))) => (&l[..], &r[..]),
            (Some(Some(l)), _) => (&l[..], "0"),
            (_, Some(Some(r))) => ("0", &r[..]),
            _ => return gantz_core::node::parse_expr("'()"),
        };
        let expr = format!("(+ {l} {r})");
        gantz_core::node::parse_expr(&expr)
    }
}
