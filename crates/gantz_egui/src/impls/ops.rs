use crate::{NodeCtx, NodeUi, Registry};

impl NodeUi for gantz_std::ops::Add {
    fn name(&self, _: &dyn Registry) -> &str {
        "+"
    }

    fn ui(
        &mut self,
        _ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui_graph::FramedResponse<egui::Response> {
        uictx.framed(|ui, _sockets| ui.add(egui::Label::new("+").selectable(false)))
    }
}
