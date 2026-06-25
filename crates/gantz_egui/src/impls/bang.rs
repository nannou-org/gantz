use crate::{NodeCtx, NodeUi, Registry, SocketDoc, SocketKind};

impl NodeUi for gantz_std::Bang {
    fn name(&self, _: &dyn Registry) -> &str {
        "!"
    }

    fn ui(
        &mut self,
        mut ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui_graph::FramedResponse<egui::Response> {
        uictx.framed(|ui, _sockets| {
            let res = ui.add(egui::Button::new(" ! "));
            if res.clicked() {
                ctx.push_eval(1);
            }
            res
        })
    }

    fn socket_doc(&self, _: &dyn Registry, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        match kind {
            SocketKind::Output => Some(
                SocketDoc::ty("bang")
                    .with_description("empty list '() emitted to trigger downstream evaluation"),
            ),
            SocketKind::Input => {
                Some(SocketDoc::ty("trigger").with_description("ignored; emits a bang when pushed"))
            }
        }
    }
}
