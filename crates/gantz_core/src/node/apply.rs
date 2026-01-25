//! A node that applies a function to a list of arguments.

use crate::node;
use gantz_ca::CaHash;
use serde::{Deserialize, Serialize};

/// A node that applies a function to arguments.
///
/// In other words, this node "calls" the function received on the first input
/// with the arguments received on the second input.
///
/// The node is stateless and evaluates immediately when a function is received.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash)]
#[cahash("gantz.apply")]
pub struct Apply;

impl<Env> node::Node<Env> for Apply {
    /// Two inputs:
    ///
    /// 1. A function value. Receiving this triggers evaluation.
    /// 2. A list of arguments. Assumed `'()` if unconnected.
    fn n_inputs(&self, _env: &Env) -> usize {
        2
    }

    /// The result of function application.
    fn n_outputs(&self, _env: &Env) -> usize {
        1
    }

    fn expr(&self, ctx: node::ExprCtx<Env>) -> node::ExprResult {
        let inputs = ctx.inputs();

        // Get function and arguments from inputs
        let function = inputs.get(0).and_then(|opt| opt.as_ref());
        let arguments = inputs.get(1).and_then(|opt| opt.as_ref());
        let args = arguments.map_or("'()", |s| &s[..]);
        let expr = function
            .map(|f| format!("(apply {f} {args})"))
            .unwrap_or_else(|| "'()".to_string());
        node::parse_expr(&expr)
    }
}
