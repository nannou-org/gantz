use crate::{NodeCtx, NodeUi};

impl<Env> NodeUi<Env> for gantz_std::Bang {
    fn name(&self, _: &Env) -> &str {
        "!"
    }

    fn ui(&mut self, mut ctx: NodeCtx<Env>, ui: &mut egui::Ui) -> egui::Response {
        let res = ui.add(egui::Button::new(" ! "));
        if res.clicked() {
            ctx.push_eval();
        }
        res
    }
}
