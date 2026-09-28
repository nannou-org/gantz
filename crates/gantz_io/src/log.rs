use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_core::steel::{
    SteelVal,
    steel_vm::{engine::Engine, register_fn::RegisterFn},
};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
        let path = ctx
            .path()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        // TODO: Switch to proper logging. Reference steel logging.scm example.
        let expr = format!("({} '({path}) {input})", fn_name(self.level));
        gantz_core::node::parse_expr(&expr)
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        // Register the default sink only if no sink is registered. Steel's
        // `register_fn` allocates a new global slot and shadows the previous
        // binding. The engine persists across recompiles, so re-registering
        // would leak the old closures.
        if ctx.vm().extract_value(fn_name(log::Level::Info)).is_err() {
            register_sink(ctx.vm(), |level, path, val| {
                log::log!(target: &log_target(path), level, "{val}");
            });
        }
    }
}

/// Register the fns that `log` nodes call. Each logged value goes to `sink`
/// with its level and the path of the node that logged it.
///
/// A `log` node registers a sink that forwards to the [`log`] crate logger,
/// but only when the VM has no sink. To capture the values that a graph's
/// `log` nodes emit, register a sink before the graph registers. Register
/// at most one sink per VM.
pub fn register_sink(
    vm: &mut Engine,
    sink: impl Fn(log::Level, &[node::Id], &SteelVal) + Send + Sync + 'static,
) {
    let sink = Arc::new(sink);
    for level in log::Level::iter() {
        let sink = sink.clone();
        vm.register_fn(fn_name(level), move |path: SteelVal, val: SteelVal| {
            sink(level, &path_from_val(&path), &val)
        });
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

/// The fn that a `log` node at the given level calls.
fn fn_name(level: log::Level) -> &'static str {
    match level {
        log::Level::Error => "log/error",
        log::Level::Warn => "log/warn",
        log::Level::Info => "log/info",
        log::Level::Debug => "log/debug",
        log::Level::Trace => "log/trace",
    }
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
        compile::{EvalKind, entry_fn_name, push_pull_entrypoints},
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

    // A sink registered before the graph receives each logged value with its
    // level and the logging node's path. The log nodes keep it rather than
    // registering their default sink.
    #[test]
    fn sink_captures_logged_values() {
        let mut g = petgraph::graph::DiGraph::new();
        let push =
            g.add_node(Box::new(node::expr("'()").unwrap().with_push_eval()) as Box<dyn Node>);
        let int = g.add_node(Box::new(node::expr("(begin $push 7)").unwrap()) as Box<_>);
        let info = g.add_node(Box::new(Log::default()) as Box<_>);
        let text = g.add_node(Box::new(node::expr("(begin $x \"hi\")").unwrap()) as Box<_>);
        let warn = g.add_node(Box::new(Log {
            level: log::Level::Warn,
        }) as Box<_>);
        g.add_edge(push, int, Edge::from((0, 0)));
        g.add_edge(int, info, Edge::from((0, 0)));
        g.add_edge(int, text, Edge::from((0, 0)));
        g.add_edge(text, warn, Edge::from((0, 0)));

        let (tx, rx) = std::sync::mpsc::channel();
        let mut vm = gantz_core::vm::new_engine(&[]);
        register_sink(&mut vm, move |level, path, val| {
            let _ = tx.send((level, path.to_vec(), val.to_string()));
        });
        gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
        let eps = push_pull_entrypoints(&no_lookup, &g);
        let config = gantz_core::compile::Config::default();
        gantz_core::vm::compile(&no_lookup, &g, &mut vm, &eps, &config)
            .unwrap_or_else(|e| panic!("compile: {}", gantz_core::vm::error_chain(&e)));
        let ep = eps
            .iter()
            .find(|ep| {
                ep.0.iter()
                    .any(|s| s.kind == EvalKind::Push && s.path == [push.index()])
            })
            .expect("push entrypoint");
        vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
            .expect("firing the push errored");

        let mut logged: Vec<_> = rx.try_iter().collect();
        logged.sort();
        assert_eq!(
            logged,
            [
                (log::Level::Warn, vec![warn.index()], "\"hi\"".to_string()),
                (log::Level::Info, vec![info.index()], "7".to_string()),
            ]
        );
    }
}
