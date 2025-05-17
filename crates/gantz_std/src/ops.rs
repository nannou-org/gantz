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

    fn expr(&self, inputs: &[Option<ExprKind>]) -> ExprKind {
        let (l, r) = match (inputs.get(0), inputs.get(1)) {
            (Some(Some(l)), Some(Some(r))) => (l.to_string(), r.to_string()),
            (Some(Some(l)), _) => (l.to_string(), "0".to_string()),
            (_, Some(Some(r))) => ("0".to_string(), r.to_string()),
            // FIXME: Need a way of handling these error cases.
            _ => return ExprKind::empty(),
        };
        let expr = format!("(+ {l} {r})");
        Engine::emit_ast(&expr).unwrap().into_iter().next().unwrap()
    }
}

#[typetag::serde]
impl gantz_core::node::SerdeNode for Add {
    fn node(&self) ->  &dyn gantz_core::Node {
        self
    }
}
