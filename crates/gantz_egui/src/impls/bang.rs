use crate::{NodeCtx, NodeUi};

impl NodeUi for gantz_std::Bang {
    fn ui(&mut self, mut ctx: NodeCtx, ui: &mut egui::Ui) -> egui::Response {
        let res = ui.add(egui::Button::new(" ! "));
        if res.clicked() {
            ctx.push_eval();
        }
        res
    }
}
