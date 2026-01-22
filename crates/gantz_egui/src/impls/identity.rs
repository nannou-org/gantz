use crate::{NodeCtx, NodeUi};

impl<Env> NodeUi<Env> for gantz_core::node::Identity {
    fn name(&self, _: &Env) -> &str {
        gantz_core::node::IDENTITY_NAME
    }

    fn ui(
        &mut self,
        _ctx: NodeCtx<Env>,
        uictx: egui_graph::NodeCtx,
    ) -> egui::InnerResponse<egui::Response> {
        uictx.framed(|ui| {
            ui.add(egui::Label::new(gantz_core::node::IDENTITY_NAME).selectable(false))
        })
    }
}
