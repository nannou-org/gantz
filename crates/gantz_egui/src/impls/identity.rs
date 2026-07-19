use crate::{Env, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind};

impl NodeUi for gantz_core::node::Identity {
    fn name(&self, _: &Env<'_>) -> std::borrow::Cow<'_, str> {
        gantz_core::node::IDENTITY_NAME.into()
    }

    fn description(&self) -> Option<&'static str> {
        Some("Pass a value through unchanged")
    }

    fn ui(&mut self, _ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let framed = uictx.framed(|ui, _sockets| {
            ui.add(egui::Label::new(gantz_core::node::IDENTITY_NAME).selectable(false))
        });
        NodeUiResponse::new(framed)
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        Some(match kind {
            SocketKind::Input => SocketDoc::ty("any").with_description("input value"),
            SocketKind::Output => {
                SocketDoc::ty("any").with_description("the input value, unchanged")
            }
        })
    }
}
