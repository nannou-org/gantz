use gantz_ca::CaHash;
use gantz_core::node::{EvalConf, ExprCtx, ExprResult, MetaCtx};
use serde::{Deserialize, Serialize};

/// A simple node for pushing evaluation through the graph.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash)]
#[cahash("gantz.bang")]
pub struct Bang;

impl gantz_core::Node for Bang {
    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
        gantz_core::node::parse_expr("'()")
    }

    fn push_eval(&self, _ctx: MetaCtx) -> Vec<EvalConf> {
        vec![EvalConf::All]
    }
}
