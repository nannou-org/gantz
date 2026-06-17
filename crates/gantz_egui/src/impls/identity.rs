use crate::{NodeCtx, NodeUi, Registry, SocketDoc, SocketKind};

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

    fn socket_doc(&self, _: &dyn Registry, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        Some(match kind {
            SocketKind::Input => SocketDoc::ty("any").with_description("input value"),
            SocketKind::Output => {
                SocketDoc::ty("any").with_description("the input value, unchanged")
            }
        })
    }
}
