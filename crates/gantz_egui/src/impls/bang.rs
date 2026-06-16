use crate::{NodeCtx, NodeUi, Registry, SocketDoc};

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

    fn output_doc(&self, _: &dyn Registry, _ix: usize) -> Option<SocketDoc> {
        Some(
            SocketDoc::ty("bang")
                .with_description("Empty list '() emitted to trigger downstream evaluation"),
        )
    }
}
