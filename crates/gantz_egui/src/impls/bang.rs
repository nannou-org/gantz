use crate::{NodeCtx, NodeUi};

impl NodeUi for gantz_std::Bang {
    fn ui(&mut self, _ctx: NodeCtx, ui: &mut egui::Ui) -> egui::Response {
        let res = ui.add(egui::Button::new(" ! "));
        if res.clicked() {
            // TODO: enqueue push eval.
            println!("BANG");
        }
        res
    }
}
