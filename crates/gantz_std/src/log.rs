use gantz_core::steel::{
    SteelVal,
    steel_vm::{engine::Engine, register_fn::RegisterFn},
};
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

impl<Env> gantz_core::Node<Env> for Log {
    fn n_inputs(&self, _: &Env) -> usize {
        1
    }

    fn expr(&self, ctx: gantz_core::node::ExprCtx<Env>) -> gantz_core::node::ExprResult {
        let Some(Some(input)) = ctx.inputs().get(0) else {
            return gantz_core::node::parse_expr("'()");
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
        gantz_core::node::parse_expr(&expr)
    }

    fn register(&self, _env: &Env, _path: &[gantz_core::node::Id], vm: &mut Engine) {
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

impl gantz_ca::CaHash for Log {
    fn hash(&self, hasher: &mut gantz_ca::Hasher) {
        format!("gantz_std::Log::{:?}", self.level).hash(hasher);
    }
}
