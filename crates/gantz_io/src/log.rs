use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_core::steel::{SteelVal, steel_vm::register_fn::RegisterFn};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};

/// A simple node that logs whatever value is received at a given log level.
///
/// The emitted expression passes the node's own path. The log entry's target
/// then identifies the emitting node. See [`log_target`].
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize, NodeTag)]
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
        // Register the helpers only if absent. Steel's `register_fn` allocates
        // a new global slot and shadows the previous binding. The engine
        // persists across recompiles, so re-registering would leak the old
        // closures.
        if ctx.vm().extract_value("log/info").is_err() {
            ctx.vm().register_fn("log/error", error);
            ctx.vm().register_fn("log/warn", warn);
            ctx.vm().register_fn("log/info", info);
            ctx.vm().register_fn("log/debug", debug);
            ctx.vm().register_fn("log/trace", trace);
        }
    }
}

/// The log target identifying the node at the given path. For example
/// `gantz:0:3:2`.
pub fn log_target(path: &[node::Id]) -> String {
    let path: Vec<String> = path.iter().map(ToString::to_string).collect();
    format!("gantz:{}", path.join(":"))
}

/// Parse the node path back out of a [`log_target`]-formatted target.
pub fn parse_log_target(target: &str) -> Option<Vec<node::Id>> {
    let path = target.strip_prefix("gantz:")?;
    path.split(':').map(|id| id.parse().ok()).collect()
}

/// The node path carried in a log fn's first argument, a quoted id list.
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
    use gantz_core::{
        Edge, Node,
        compile::push_pull_entrypoints,
        node::{self, WithPushEval},
    };

    fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
        None
    }

    #[test]
    fn log_target_roundtrip() {
        for path in [vec![0], vec![3, 2, 1], vec![10, 200]] {
            assert_eq!(parse_log_target(&log_target(&path)), Some(path));
        }
    }

    #[test]
    fn non_gantz_targets_rejected() {
        for target in ["", "gantz_io::log", "gantz:", "gantz:x", "gantz:1:x"] {
            assert_eq!(parse_log_target(target), None, "{target}");
        }
    }

    // The emitted module passes the log node's path as a quoted literal to the
    // registered log fn, so log entries identify their emitting node.
    #[test]
    fn log_expr_carries_node_path() {
        let mut g = petgraph::graph::DiGraph::new();
        let push =
            g.add_node(Box::new(node::expr("'()").unwrap().with_push_eval()) as Box<dyn Node>);
        let int = g.add_node(Box::new(node::expr("(begin $push 7)").unwrap()) as Box<_>);
        let log = g.add_node(Box::new(Log::default()) as Box<_>);
        g.add_edge(push, int, Edge::from((0, 0)));
        g.add_edge(int, log, Edge::from((0, 0)));

        let eps = push_pull_entrypoints(&no_lookup, &g);
        let module =
            gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();
        let src = gantz_core::vm::fmt_module(&module);
        let expected = format!("(log/info (quote ({})) ", log.index());
        assert!(
            src.contains(&expected) || src.contains(&format!("(log/info '({})", log.index())),
            "module does not pass the log node's path:\n{src}"
        );
    }
}
