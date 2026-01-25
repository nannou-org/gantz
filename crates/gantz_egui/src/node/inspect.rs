//! An Inspect node for viewing SteelVals flowing through the graph.

use crate::{NodeCtx, NodeUi};
use gantz_ca::CaHash;
use gantz_core::node;
use serde::{Deserialize, Serialize};
use steel::steel_vm::engine::Engine;

/// A node that displays the debug representation of values passing through.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash)]
#[cahash("gantz.inspect")]
pub struct Inspect;

impl<Env> gantz_core::Node<Env> for Inspect {
    fn n_inputs(&self, _: &Env) -> usize {
        1
    }

    fn n_outputs(&self, _: &Env) -> usize {
        1
    }

    fn stateful(&self, _: &Env) -> bool {
        true
    }

    fn expr(&self, ctx: node::ExprCtx<Env>) -> node::ExprResult {
        let expr = match ctx.inputs().get(0) {
            Some(Some(val)) => format!("(begin (set! state {val}) state)"),
            _ => "(begin state)".to_string(),
        };
        node::parse_expr(&expr)
    }

    fn register(&self, _env: &Env, path: &[node::Id], vm: &mut Engine) {
        node::state::init_value_if_absent(vm, path, || steel::SteelVal::Void).unwrap()
    }
}

impl<Env> NodeUi<Env> for Inspect {
    fn name(&self, _: &Env) -> &str {
        "inspect"
    }

    fn ui(
        &mut self,
        ctx: NodeCtx<Env>,
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
