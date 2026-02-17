use crate::{NodeCtx, NodeUi, Registry};

impl NodeUi for gantz_core::node::Apply {
    fn name(&self, _: &dyn Registry) -> &str {
        "apply"
    }

    fn ui(
        &mut self,
        _ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui::InnerResponse<egui::Response> {
        uictx.framed(|ui| ui.add(egui::Label::new("apply").selectable(false)))
    }
}
