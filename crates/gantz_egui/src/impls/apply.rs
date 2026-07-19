use crate::{Env, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind};

impl NodeUi for gantz_core::node::Apply {
    fn name(&self, _: &Env<'_>) -> std::borrow::Cow<'_, str> {
        "apply".into()
    }

    fn description(&self) -> Option<&'static str> {
        Some("Apply a function to arguments")
    }

    fn ui(&mut self, _ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let framed =
            uictx.framed(|ui, _sockets| ui.add(egui::Label::new("apply").selectable(false)));
        NodeUiResponse::new(framed)
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, ix: usize) -> Option<SocketDoc> {
        match (kind, ix) {
            (SocketKind::Input, 0) => Some(
                SocketDoc::ty("function")
                    .with_description("function to apply (receiving here triggers evaluation)"),
            ),
            (SocketKind::Input, 1) => {
                Some(SocketDoc::ty("list").with_description("argument list ('() if unconnected)"))
            }
            (SocketKind::Output, _) => {
                Some(SocketDoc::ty("any").with_description("result of applying the function"))
            }
            _ => None,
        }
    }
}
