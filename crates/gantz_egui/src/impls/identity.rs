use crate::{NodeCtx, NodeUi, Registry};

impl NodeUi for gantz_core::node::Identity {
    fn name(&self, _: &dyn Registry) -> &str {
        gantz_core::node::IDENTITY_NAME
    }

    fn ui(
        &mut self,
        _ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui::InnerResponse<egui::Response> {
        uictx.framed(|ui| {
            ui.add(egui::Label::new(gantz_core::node::IDENTITY_NAME).selectable(false))
        })
    }
}
