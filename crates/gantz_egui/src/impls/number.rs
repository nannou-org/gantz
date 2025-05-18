use steel::SteelVal;

use crate::{NodeCtx, NodeUi};

impl NodeUi for gantz_std::number::Number {
    fn ui(&mut self, mut ctx: NodeCtx, ui: &mut egui::Ui) -> egui::Response {
        let mut val = ctx.extract().unwrap().unwrap_or(SteelVal::NumV(0.0));
        let res = match val {
            SteelVal::NumV(ref mut f) => ui.add(egui::DragValue::new(f)),
            SteelVal::IntV(ref mut i) => ui.add(egui::DragValue::new(i)),
            _ => ui.add(egui::Label::new("ERR")),
        };
        if res.changed() {
            ctx.register(val).unwrap();
            ctx.push_eval();
        }
        res
    }
}
