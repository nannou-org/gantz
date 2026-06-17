use crate::{NodeCtx, NodeUi, Registry, SocketDoc, SocketKind};

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

    fn socket_doc(&self, _: &dyn Registry, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        Some(match kind {
            SocketKind::Input => {
                SocketDoc::ty("any").with_description("value stored for the next evaluation")
            }
            SocketKind::Output => SocketDoc::ty("any").with_description(
                "value from the previous evaluation (initially '()); enables feedback cycles",
            ),
        })
    }
}
