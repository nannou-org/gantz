use gantz_core::steel::{parser::ast::ExprKind, steel_vm::engine::Engine};
use serde::{Deserialize, Serialize};

/// A simple node that logs whatever value is received at a given log level.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct Log {
    pub level: log::Level,
}

impl Default for Log {
    fn default() -> Self {
        Self {
            level: log::Level::Info,
        }
    }
}

impl gantz_core::Node for Log {
    fn n_inputs(&self) -> usize {
        1
    }

    fn expr(&self, inputs: &[Option<ExprKind>]) -> ExprKind {
        let Some(Some(input)) = inputs.get(0) else {
            return ExprKind::empty();
        };
        let level = match self.level {
            log::Level::Error => "error",
            log::Level::Warn => "warn",
            log::Level::Info => "info",
            log::Level::Debug => "debug",
            log::Level::Trace => "trace",
        };
        let expr = format!("(log/{level}! {input})");
        Engine::emit_ast(&expr).unwrap().into_iter().next().unwrap()
    }
}

#[typetag::serde]
impl gantz_core::node::SerdeNode for Log {
    fn node(&self) ->  &dyn gantz_core::Node {
        self
    }
}
