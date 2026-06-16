use crate::{NodeCtx, NodeUi, Registry, SocketDoc};

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

    fn input_doc(&self, _: &dyn Registry, ix: usize) -> Option<SocketDoc> {
        match ix {
            0 => Some(SocketDoc::ty("number").with_description("Left addend (0 if unconnected)")),
            1 => Some(SocketDoc::ty("number").with_description("Right addend (0 if unconnected)")),
            _ => None,
        }
    }

    fn output_doc(&self, _: &dyn Registry, _ix: usize) -> Option<SocketDoc> {
        Some(SocketDoc::ty("number").with_description("Sum of the two inputs"))
    }
}
