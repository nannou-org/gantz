//! An Inspect node for viewing SteelVals flowing through the graph.

use crate::{Env, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind, ui_tree::UiTree};
use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};

/// A node that displays the debug representation of values passing through.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize, NodeTag)]
pub struct Inspect;

impl gantz_core::Node for Inspect {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        true
    }

    fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult {
        let expr = match ctx.inputs().get(0) {
            Some(Some(val)) => format!("(begin (set! state {val}) state)"),
            _ => "(begin state)".to_string(),
        };
        node::parse_expr(&expr)
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        let path = ctx.path();
        node::state::init_value_if_absent(ctx.vm(), path, || steel::SteelVal::Void).unwrap()
    }
}

impl NodeUi for Inspect {
    fn name(&self, _: &crate::Env<'_>) -> std::borrow::Cow<'_, str> {
        "inspect".into()
    }

    fn ui(&mut self, mut ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let mut frame = egui_graph::node::default_frame(uictx.style(), uictx.interaction());
        frame.fill = uictx.style().visuals.extreme_bg_color;
        let (&id, prefix) = ctx.path().split_last().expect("a node path is never empty");
        let tree = fragment(id);
        let root_id = uictx.egui_id().with("gui");
        let framed = uictx.framed_with(frame, |ui, _sockets| {
            let r = UiTree::new(root_id)
                .instance_prefix(prefix)
                .show(&tree, &mut ctx, ui);
            r.inner.unwrap_or_else(|| ui.response())
        });
        NodeUiResponse::new(framed)
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        Some(match kind {
            SocketKind::Input => {
                SocketDoc::ty("any").with_description("value to display; stored and passed through")
            }
            SocketKind::Output => {
                SocketDoc::ty("any").with_description("the input value, unchanged")
            }
        })
    }
}

/// The inspect's value fragment: a read-only repr of its own state.
fn fragment(id: node::Id) -> gantz_ui::Element {
    gantz_ui::Element::Value(gantz_ui::Value {
        bind: Some(gantz_ui::BindPath(vec![id])),
        wrap: false,
        key: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragment_bakes_bind() {
        let expected = gantz_ui::Element::Value(gantz_ui::Value {
            bind: Some(gantz_ui::BindPath(vec![2])),
            wrap: false,
            key: None,
        });
        assert_eq!(fragment(2), expected);
    }
}
