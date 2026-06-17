use crate::{NodeCtx, NodeUi, Registry, SocketDoc, SocketKind};

impl NodeUi for gantz_std::log::Log {
    fn name(&self, _: &dyn Registry) -> &str {
        match self.level {
            log::Level::Error => "error",
            log::Level::Warn => "warn",
            log::Level::Info => "info",
            log::Level::Debug => "debug",
            log::Level::Trace => "trace",
        }
    }

    fn ui(
        &mut self,
        _ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui_graph::FramedResponse<egui::Response> {
        uictx.framed(|ui, _sockets| {
            let level = format!("{:?}", self.level).to_lowercase();
            ui.add(egui::Label::new(&level).selectable(false))
        })
    }

    fn socket_doc(&self, _: &dyn Registry, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        match kind {
            SocketKind::Input => {
                Some(SocketDoc::ty("any").with_description("value logged at this node's level"))
            }
            SocketKind::Output => None,
        }
    }
}
