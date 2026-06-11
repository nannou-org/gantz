use crate::{NodeCtx, NodeUi, Registry};

impl NodeUi for gantz_core::node::Delay {
    fn name(&self, _: &dyn Registry) -> &str {
        "delay"
    }

    fn ui(
        &mut self,
        _ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui_graph::FramedResponse<egui::Response> {
        uictx.framed(|ui, _sockets| ui.add(egui::Label::new("delay").selectable(false)))
    }
}
