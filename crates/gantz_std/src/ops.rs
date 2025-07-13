use gantz_core::steel::{parser::ast::ExprKind, steel_vm::engine::Engine};
use serde::{Deserialize, Serialize};

/// Simple `Add` operation node.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct Add;

impl gantz_core::Node for Add {
    fn n_inputs(&self) -> usize {
        2
    }

    fn n_outputs(&self) -> usize {
        1
    }

    fn expr(&self, ctx: gantz_core::node::ExprCtx) -> ExprKind {
        let inputs = ctx.inputs();
        let (l, r) = match (inputs.get(0), inputs.get(1)) {
            (Some(Some(l)), Some(Some(r))) => (&l[..], &r[..]),
            (Some(Some(l)), _) => (&l[..], "0"),
            (_, Some(Some(r))) => ("0", &r[..]),
            // FIXME: Need a way of handling these error cases.
            _ => return ExprKind::empty(),
        };
        let expr = format!("(+ {l} {r})");
        Engine::emit_ast(&expr).unwrap().into_iter().next().unwrap()
    }
}
