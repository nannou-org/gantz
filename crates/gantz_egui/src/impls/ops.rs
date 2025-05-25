use crate::{NodeCtx, NodeUi};

impl NodeUi for gantz_std::ops::Add {
    fn name(&self) -> &str {
        "+"
    }

    fn ui(&mut self, _ctx: NodeCtx, ui: &mut egui::Ui) -> egui::Response {
        ui.add(egui::Label::new("+"))
    }
}
