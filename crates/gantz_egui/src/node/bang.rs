//! A bang node: a dot that fires downstream evaluation.

use crate::{Env, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind, widget::kit};
use gantz_core::node::{self, EvalConf, ExprCtx, ExprResult, MetaCtx};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};

/// A simple node for pushing evaluation through the graph.
///
/// Exposes a single trigger input whose value is ignored. A push into it
/// emits a bang, `'()`, downstream. This lets upstream nodes fire the bang as
/// well as a click on its dot. Any input value becomes a bang.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize, NodeTag)]
pub struct Bang;

impl gantz_core::Node for Bang {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
        node::parse_expr("'()")
    }

    fn push_eval(&self, _ctx: MetaCtx) -> Vec<EvalConf> {
        vec![EvalConf::All]
    }
}

impl NodeUi for Bang {
    fn name(&self, _: &Env<'_>) -> std::borrow::Cow<'_, str> {
        "!".into()
    }

    fn description(&self) -> Option<&'static str> {
        Some("Trigger downstream evaluation")
    }

    fn ui(&mut self, ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        // A bang only triggers downstream evaluation. It never edits the
        // node's content address, so a click pushes but never sets `changed`.
        let dot_id = uictx.egui_id().with("dot");
        let framed =
            uictx.framed(|ui, _sockets| kit::event_label(ui, dot_id, None, egui::Sense::click()));
        let clicked = framed.inner.inner.clicked();
        if clicked {
            kit::fire_dot(&framed.inner.response.ctx, dot_id);
        }
        let mut resp = NodeUiResponse::new(framed);
        if clicked {
            resp.push_eval(ctx.path(), 1);
        }
        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        match kind {
            SocketKind::Output => Some(
                SocketDoc::ty("bang")
                    .with_description("empty list '() emitted to trigger downstream evaluation"),
            ),
            SocketKind::Input => Some(
                SocketDoc::ty("trigger").with_description("ignored. A push into it emits a bang"),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_core::{
        Edge, Node,
        compile::{EvalKind, entry_fn_name, push_pull_entrypoints},
        node::WithPushEval,
    };

    fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
        None
    }

    // Firing a value into a bang's trigger input emits `'()` downstream. The
    // downstream `check` asserts it received an empty list, so a successful fire
    // proves the bang ignored its input and emitted a bang.
    #[test]
    fn bang_trigger_input_emits_bang() {
        let mut g = petgraph::graph::DiGraph::new();
        let push =
            g.add_node(Box::new(node::expr("'()").unwrap().with_push_eval()) as Box<dyn Node>);
        // A constant value, fired by the push via its own trigger input.
        let val = g.add_node(Box::new(node::expr("42").unwrap()) as Box<_>);
        // The bang ignores `val`'s output and emits `'()`.
        let bang = g.add_node(Box::new(Bang) as Box<_>);
        let check =
            g.add_node(Box::new(node::expr("(assert! (equal? $b '()))").unwrap()) as Box<_>);
        g.add_edge(push, val, Edge::from((0, 0)));
        g.add_edge(val, bang, Edge::from((0, 0)));
        g.add_edge(bang, check, Edge::from((0, 0)));

        let config = gantz_core::compile::Config::default();
        let eps = push_pull_entrypoints(&no_lookup, &g);
        let (mut vm, _compiled) = gantz_core::vm::init(&no_lookup, &g, &eps, &config)
            .unwrap_or_else(|e| panic!("init: {}", gantz_core::vm::error_chain(&e)));

        let ep = eps
            .iter()
            .find(|ep| {
                ep.0.iter()
                    .any(|s| s.kind == EvalKind::Push && s.path == [push.index()])
            })
            .expect("push entrypoint");
        vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
            .expect("firing bang trigger errored");
    }
}
