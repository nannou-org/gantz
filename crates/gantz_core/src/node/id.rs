//! A node representing the identity function - returns its input unchanged.

use crate::node;
use gantz_ca::CaHash;
use serde::{Deserialize, Serialize};

/// The name used for the identity node in the registry.
pub const IDENTITY_NAME: &str = "id";

/// The identity function - a pure function that returns its input unchanged.
///
/// This is a fundamental building block in functional programming,
/// often used as a default or no-op function.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash)]
#[cahash("gantz.identity")]
pub struct Identity;

impl<Env> node::Node<Env> for Identity {
    fn n_inputs(&self, _env: &Env) -> usize {
        1
    }

    fn n_outputs(&self, _env: &Env) -> usize {
        1
    }

    fn expr(&self, ctx: node::ExprCtx<Env>) -> node::ExprResult {
        let inputs = ctx.inputs();

        // Simply return the input unchanged
        let expr = match inputs.get(0) {
            Some(Some(input)) => input.clone(),
            _ => "'()".to_string(),
        };

        node::parse_expr(&expr)
    }
}
