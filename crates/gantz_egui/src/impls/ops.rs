use crate::{NodeCtx, NodeUi};

impl<Env> NodeUi<Env> for gantz_std::ops::Add {
    fn name(&self, _: &Env) -> &str {
        "+"
    }

    fn ui(&mut self, _ctx: NodeCtx<Env>, ui: &mut egui::Ui) -> egui::Response {
        ui.add(egui::Label::new("+").selectable(false))
    }
}
