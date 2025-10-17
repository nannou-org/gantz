use crate::{NodeCtx, NodeUi};

impl<Env> NodeUi<Env> for gantz_core::node::graph::Inlet {
    fn name(&self, _: &Env) -> &str {
        "in"
    }

    fn ui(&mut self, ctx: NodeCtx<Env>, ui: &mut egui::Ui) -> egui::Response {
        ui.add(egui::Label::new(self.name(ctx.env())).selectable(false))
    }
}
