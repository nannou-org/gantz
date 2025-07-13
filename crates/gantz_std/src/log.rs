use gantz_core::steel::{
    parser::ast::ExprKind,
    steel_vm::{engine::Engine, register_fn::RegisterFn},
};
use serde::{Deserialize, Serialize};
use steel::SteelVal;

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

    fn expr(&self, ctx: gantz_core::node::ExprCtx) -> ExprKind {
        let Some(Some(input)) = ctx.inputs().get(0) else {
            return ExprKind::empty();
        };
        let level = match self.level {
            log::Level::Error => "error",
            log::Level::Warn => "warn",
            log::Level::Info => "info",
            log::Level::Debug => "debug",
            log::Level::Trace => "trace",
        };
        // TODO: Switch to proper logging. Reference steel logging.scm example.
        let expr = format!("(log/{level} {input})");
        Engine::emit_ast(&expr).unwrap().into_iter().next().unwrap()
    }

    fn register(&self, _path: &[gantz_core::node::Id], vm: &mut Engine) {
        fn error(val: SteelVal) {
            log::error!("{}", val);
        }
        fn warn(val: SteelVal) {
            log::warn!("{}", val);
        }
        fn info(val: SteelVal) {
            log::info!("{}", val);
        }
        fn debug(val: SteelVal) {
            log::debug!("{}", val);
        }
        fn trace(val: SteelVal) {
            log::trace!("{}", val);
        }
        vm.register_fn("log/error", error);
        vm.register_fn("log/warn", warn);
        vm.register_fn("log/info", info);
        vm.register_fn("log/debug", debug);
        vm.register_fn("log/trace", trace);
    }
}
