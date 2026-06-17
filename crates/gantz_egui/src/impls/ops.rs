use crate::{NodeCtx, NodeUi, Registry, SocketDoc, SocketKind};

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

    fn socket_doc(&self, _: &dyn Registry, kind: SocketKind, ix: usize) -> Option<SocketDoc> {
        match (kind, ix) {
            (SocketKind::Input, 0) => {
                Some(SocketDoc::ty("number").with_description("left addend (0 if unconnected)"))
            }
            (SocketKind::Input, 1) => {
                Some(SocketDoc::ty("number").with_description("right addend (0 if unconnected)"))
            }
            (SocketKind::Output, _) => {
                Some(SocketDoc::ty("number").with_description("sum of the two inputs"))
            }
            _ => None,
        }
    }
}
