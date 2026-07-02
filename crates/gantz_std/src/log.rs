use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_core::steel::{SteelVal, steel_vm::register_fn::RegisterFn};
use serde::{Deserialize, Serialize};

/// A simple node that logs whatever value is received at a given log level.
///
/// The emitted expression passes the node's own path so the log entry's
/// target identifies the emitting node (see [`log_target`]).
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct Log {
    pub level: log::Level,
}

impl gantz_format::NodeTag for Log {
    const TAG: &'static str = "Log";
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
        let path = ctx
            .path()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        // TODO: Switch to proper logging. Reference steel logging.scm example.
        let expr = format!("(log/{level} '({path}) {input})");
        gantz_core::node::parse_expr(&expr)
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        fn log_val(level: log::Level, path: &SteelVal, val: &SteelVal) {
            let path = path_from_val(path);
            log::log!(target: &log_target(&path), level, "{val}");
        }
        fn error(path: SteelVal, val: SteelVal) {
            log_val(log::Level::Error, &path, &val);
        }
        fn warn(path: SteelVal, val: SteelVal) {
            log_val(log::Level::Warn, &path, &val);
        }
        fn info(path: SteelVal, val: SteelVal) {
            log_val(log::Level::Info, &path, &val);
        }
        fn debug(path: SteelVal, val: SteelVal) {
            log_val(log::Level::Debug, &path, &val);
        }
        fn trace(path: SteelVal, val: SteelVal) {
            log_val(log::Level::Trace, &path, &val);
        }
        // Register the helpers only if absent. Steel's `register_fn` allocates a
        // new global slot and shadows the previous binding rather than
        // overwriting it, so re-registering on every recompile (the engine
        // persists across them) would leak the old closures.
        if ctx.vm().extract_value("log/info").is_err() {
            ctx.vm().register_fn("log/error", error);
            ctx.vm().register_fn("log/warn", warn);
            ctx.vm().register_fn("log/info", info);
            ctx.vm().register_fn("log/debug", debug);
            ctx.vm().register_fn("log/trace", trace);
        }
    }
}

impl gantz_ca::CaHash for Log {
    fn hash(&self, hasher: &mut gantz_ca::Hasher) {
        format!("gantz_std::Log::{:?}", self.level).hash(hasher);
    }
}

/// The log target identifying the node at the given path, e.g. `gantz:0:3:2`.
pub fn log_target(path: &[node::Id]) -> String {
    let path: Vec<String> = path.iter().map(ToString::to_string).collect();
    format!("gantz:{}", path.join(":"))
}

/// Parse the node path back out of a [`log_target`]-formatted target.
pub fn parse_log_target(target: &str) -> Option<Vec<node::Id>> {
    let path = target.strip_prefix("gantz:")?;
    path.split(':').map(|id| id.parse().ok()).collect()
}

/// The node path carried in a log fn's first argument (a quoted id list).
fn path_from_val(val: &SteelVal) -> Vec<node::Id> {
    match val {
        SteelVal::ListV(ids) => ids
            .iter()
            .filter_map(|id| match id {
                SteelVal::IntV(id) => usize::try_from(*id).ok(),
                _ => None,
            })
            .collect(),
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_target_roundtrip() {
        for path in [vec![0], vec![3, 2, 1], vec![10, 200]] {
            assert_eq!(parse_log_target(&log_target(&path)), Some(path));
        }
    }

    #[test]
    fn non_gantz_targets_rejected() {
        for target in ["", "gantz_std::log", "gantz:", "gantz:x", "gantz:1:x"] {
            assert_eq!(parse_log_target(target), None, "{target}");
        }
    }
}
