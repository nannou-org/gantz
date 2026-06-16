use crate::{NodeCtx, NodeUi, Registry, SocketDoc};

impl NodeUi for gantz_core::node::Apply {
    fn name(&self, _: &dyn Registry) -> &str {
        "apply"
    }

    fn ui(
        &mut self,
        _ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui_graph::FramedResponse<egui::Response> {
        uictx.framed(|ui, _sockets| ui.add(egui::Label::new("apply").selectable(false)))
    }

    fn input_doc(&self, _: &dyn Registry, ix: usize) -> Option<SocketDoc> {
        match ix {
            0 => Some(
                SocketDoc::ty("function")
                    .with_description("Function to apply (receiving here triggers evaluation)"),
            ),
            1 => Some(SocketDoc::ty("list").with_description("Argument list ('() if unconnected)")),
            _ => None,
        }
    }

    fn output_doc(&self, _: &dyn Registry, _ix: usize) -> Option<SocketDoc> {
        Some(SocketDoc::ty("any").with_description("Result of applying the function"))
    }
}
