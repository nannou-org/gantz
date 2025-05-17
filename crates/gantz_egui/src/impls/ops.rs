use crate::{NodeCtx, NodeUi};

impl NodeUi for gantz_std::ops::Add {
    fn ui(&mut self, _ctx: NodeCtx, ui: &mut egui::Ui) -> egui::Response {
        ui.add(egui::Label::new("+"))
    }
}
