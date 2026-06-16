use crate::{NodeCtx, NodeUi, Registry, SocketDoc};

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

    fn input_doc(&self, _: &dyn Registry, _ix: usize) -> Option<SocketDoc> {
        Some(SocketDoc::ty("any").with_description("Value stored for the next evaluation"))
    }

    fn output_doc(&self, _: &dyn Registry, _ix: usize) -> Option<SocketDoc> {
        Some(SocketDoc::ty("any").with_description(
            "Value from the previous evaluation (initially '()); enables feedback cycles",
        ))
    }
}
