//! An Inspect node for viewing SteelVals flowing through the graph.

use crate::{NodeCtx, NodeUi};
use gantz_ca::CaHash;
use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx, RegCtx};
use serde::{Deserialize, Serialize};

/// A node that displays the debug representation of values passing through.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash)]
#[cahash("gantz.inspect")]
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
    fn name(&self, _: &dyn crate::Registry) -> &str {
        "inspect"
    }

    fn ui(
        &mut self,
        ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui::InnerResponse<egui::Response> {
        let mut frame = egui_graph::node::default_frame(uictx.style(), uictx.interaction());
        frame.fill = uictx.style().visuals.extreme_bg_color;
        uictx.framed_with(frame, |ui| {
            let text = match ctx.extract_value() {
                Ok(Some(val)) => format!("{:?}", val),
                Ok(None) => "∅".to_string(),
                Err(_) => "ERR".to_string(),
            };
            ui.add(egui::Label::new(&text).selectable(false))
        })
    }
}
