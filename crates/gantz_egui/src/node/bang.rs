//! A bang node with a button for triggering downstream evaluation.

use crate::{Env, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind, ui_tree::UiTree};
use gantz_core::node::{self, EvalConf, ExprCtx, ExprResult, MetaCtx};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};

/// A simple node for pushing evaluation through the graph.
///
/// Exposes a single trigger input whose value is ignored. A push into it
/// emits a bang, `'()`, downstream. This lets upstream nodes fire the bang as
/// well as its button. Any input value becomes a bang.
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

    fn ui(&mut self, mut ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        // A bang only triggers downstream evaluation. It never edits the
        // node's content address, so the button pushes but never sets
        // `changed`.
        let (&id, prefix) = ctx.path().split_last().expect("a node path is never empty");
        let tree = fragment(id);
        let root_id = uictx.egui_id().with("gui");
        let mut payloads = Vec::new();
        let framed = uictx.framed(|ui, _sockets| {
            let r = UiTree::new(root_id)
                .instance_prefix(prefix)
                .n_outputs(&|_: &[node::Id]| Some(1))
                .show(&tree, &mut ctx, ui);
            payloads = r.payloads;
            r.inner.unwrap_or_else(|| ui.response())
        });
        let mut resp = NodeUiResponse::new(framed);
        resp.payloads.extend(payloads);
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

/// The bang's button fragment, bound to its own id. The padded label is the
/// bang's visual, not an interpreter default.
fn fragment(id: node::Id) -> gantz_ui::Element {
    gantz_ui::Element::Button(gantz_ui::Button {
        bind: Some(gantz_ui::BindPath(vec![id])),
        label: Some(" ! ".to_string()),
        key: None,
    })
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
