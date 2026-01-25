use gantz_ca::CaHash;
use serde::{Deserialize, Serialize};

/// A simple node for pushing evaluation through the graph.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash)]
#[cahash("gantz.bang")]
pub struct Bang;

impl<Env> gantz_core::Node<Env> for Bang {
    fn n_outputs(&self, _: &Env) -> usize {
        1
    }

    fn expr(&self, _ctx: gantz_core::node::ExprCtx<Env>) -> gantz_core::node::ExprResult {
        gantz_core::node::parse_expr("'()")
    }

    fn push_eval(&self, _: &Env) -> Vec<gantz_core::node::EvalConf> {
        vec![gantz_core::node::EvalConf::All]
    }
}
