use crate::{NodeCtx, NodeUi};

impl NodeUi for gantz_core::node::graph::Outlet {
    fn name(&self) -> &str {
        "out"
    }

    fn ui(&mut self, _ctx: NodeCtx, ui: &mut egui::Ui) -> egui::Response {
        ui.add(egui::Label::new(self.name()).selectable(false))
    }
}
