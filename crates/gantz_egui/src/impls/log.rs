use crate::{NodeCtx, NodeUi};

impl<Env> NodeUi<Env> for gantz_std::log::Log {
    fn name(&self, _: &Env) -> &str {
        match self.level {
            log::Level::Error => "error",
            log::Level::Warn => "warn",
            log::Level::Info => "info",
            log::Level::Debug => "debug",
            log::Level::Trace => "trace",
        }
    }

    fn ui(&mut self, _ctx: NodeCtx<Env>, ui: &mut egui::Ui) -> egui::Response {
        let level = format!("{:?}", self.level).to_lowercase();
        ui.add(egui::Label::new(&level).selectable(false))
    }
}
