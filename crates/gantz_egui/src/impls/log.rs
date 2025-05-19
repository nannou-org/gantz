use crate::{NodeCtx, NodeUi};

impl NodeUi for gantz_std::log::Log {
    fn ui(&mut self, _ctx: NodeCtx, ui: &mut egui::Ui) -> egui::Response {
        let level = format!("{:?}", self.level).to_lowercase();
        ui.add(egui::Label::new(&level))
    }
}
