use crate::{NodeCtx, NodeUi, Registry};

impl NodeUi for gantz_std::Bang {
    fn name(&self, _: &dyn Registry) -> &str {
        "!"
    }

    fn ui(
        &mut self,
        mut ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui::InnerResponse<egui::Response> {
        uictx.framed(|ui| {
            let res = ui.add(egui::Button::new(" ! "));
            if res.clicked() {
                ctx.push_eval();
            }
            res
        })
    }
}
