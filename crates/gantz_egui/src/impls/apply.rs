use crate::{NodeCtx, NodeUi};

impl<Env> NodeUi<Env> for gantz_core::node::Apply {
    fn name(&self, _: &Env) -> &str {
        "apply"
    }

    fn ui(
        &mut self,
        _ctx: NodeCtx<Env>,
        uictx: egui_graph::NodeCtx,
    ) -> egui::InnerResponse<egui::Response> {
        uictx.framed(|ui| {
            ui.add(egui::Label::new("apply").selectable(false))
        })
    }
}