//! The identity function - returns its input unchanged.

use crate::node;
use serde::{Deserialize, Serialize};
use steel::{parser::ast::ExprKind, steel_vm::engine::Engine};

/// The name used for the identity node in the registry.
pub const IDENTITY_NAME: &str = "id";

/// The identity function - a pure function that returns its input unchanged.
///
/// This is a fundamental building block in functional programming,
/// often used as a default or no-op function.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct Identity;

impl<Env> node::Node<Env> for Identity {
    fn n_inputs(&self, _env: &Env) -> usize {
        1
    }

    fn n_outputs(&self, _env: &Env) -> usize {
        1
    }

    fn expr(&self, ctx: node::ExprCtx<Env>) -> ExprKind {
        let inputs = ctx.inputs();

        // Simply return the input unchanged
        let expr = match inputs.get(0) {
            Some(Some(input)) => input.clone(),
            _ => "'()".to_string(),
        };

        Engine::emit_ast(&expr).unwrap().into_iter().next().unwrap()
    }
}

impl gantz_ca::CaHash for Identity {
    fn hash(&self, hasher: &mut gantz_ca::Hasher) {
        "gantz_core::node::Identity".hash(hasher);
    }
}
