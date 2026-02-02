use gantz_core::node::{ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_core::steel::{SteelVal, steel_vm::register_fn::RegisterFn};
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
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult {
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

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
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
        ctx.vm().register_fn("log/error", error);
        ctx.vm().register_fn("log/warn", warn);
        ctx.vm().register_fn("log/info", info);
        ctx.vm().register_fn("log/debug", debug);
        ctx.vm().register_fn("log/trace", trace);
    }
}

impl gantz_ca::CaHash for Log {
    fn hash(&self, hasher: &mut gantz_ca::Hasher) {
        format!("gantz_std::Log::{:?}", self.level).hash(hasher);
    }
}
