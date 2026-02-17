use crate::{NodeCtx, NodeUi, Registry};

impl NodeUi for gantz_std::ops::Add {
    fn name(&self, _: &dyn Registry) -> &str {
        "+"
    }

    fn ui(
        &mut self,
        _ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui::InnerResponse<egui::Response> {
        uictx.framed(|ui| ui.add(egui::Label::new("+").selectable(false)))
    }
}
