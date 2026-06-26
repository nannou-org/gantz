use crate::{NodeCtx, NodeUi, NodeUiResponse, Registry, SocketDoc, SocketKind};

impl NodeUi for gantz_std::Bang {
    fn name(&self, _: &dyn Registry) -> &str {
        "!"
    }

    fn description(&self) -> Option<&'static str> {
        Some("Trigger downstream evaluation")
    }

    fn ui(&mut self, ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        // A bang only triggers downstream evaluation; it never edits the
        // node's content address, so we queue an eval but never mark `changed`.
        let mut clicked = false;
        let framed = uictx.framed(|ui, _sockets| {
            let res = ui.add(egui::Button::new(" ! "));
            clicked = res.clicked();
            res
        });
        let mut resp = NodeUiResponse::new(framed);
        if clicked {
            resp.push_eval(ctx.path(), 1);
        }
        resp
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
