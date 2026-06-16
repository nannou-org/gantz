use crate::{NodeCtx, NodeUi, Registry, SocketDoc};

impl NodeUi for gantz_core::node::Identity {
    fn name(&self, _: &dyn Registry) -> &str {
        gantz_core::node::IDENTITY_NAME
    }

    fn ui(
        &mut self,
        _ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui_graph::FramedResponse<egui::Response> {
        uictx.framed(|ui, _sockets| {
            ui.add(egui::Label::new(gantz_core::node::IDENTITY_NAME).selectable(false))
        })
    }

    fn input_doc(&self, _: &dyn Registry, _ix: usize) -> Option<SocketDoc> {
        Some(SocketDoc::ty("any").with_description("Input value"))
    }

    fn output_doc(&self, _: &dyn Registry, _ix: usize) -> Option<SocketDoc> {
        Some(SocketDoc::ty("any").with_description("The input value, unchanged"))
    }
}
