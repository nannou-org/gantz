use crate::{NodeCtx, NodeUi, Registry};

impl NodeUi for gantz_std::log::Log {
    fn name(&self, _: &dyn Registry) -> &str {
        match self.level {
            log::Level::Error => "error",
            log::Level::Warn => "warn",
            log::Level::Info => "info",
            log::Level::Debug => "debug",
            log::Level::Trace => "trace",
        }
    }

    fn ui(
        &mut self,
        _ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui::InnerResponse<egui::Response> {
        uictx.framed(|ui| {
            let level = format!("{:?}", self.level).to_lowercase();
            ui.add(egui::Label::new(&level).selectable(false))
        })
    }
}
