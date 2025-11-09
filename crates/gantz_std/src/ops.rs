use gantz_core::{
    ca::CaHash,
    steel::{parser::ast::ExprKind, steel_vm::engine::Engine},
};
use serde::{Deserialize, Serialize};

/// Simple `Add` operation node.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct Add;

impl<Env> gantz_core::Node<Env> for Add {
    fn n_inputs(&self, _: &Env) -> usize {
        2
    }

    fn n_outputs(&self, _: &Env) -> usize {
        1
    }

    fn expr(&self, ctx: gantz_core::node::ExprCtx<Env>) -> ExprKind {
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

impl CaHash for Add {
    fn hash(&self, hasher: &mut gantz_core::ca::Hasher) {
        "gantz_std::Add".hash(hasher);
    }
}
