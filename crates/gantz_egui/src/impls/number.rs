use crate::{NodeCtx, NodeUi};
use steel::SteelVal;

impl NodeUi for gantz_std::number::Number {
    fn name(&self) -> &str {
        "number"
    }

    fn ui(&mut self, mut ctx: NodeCtx, ui: &mut egui::Ui) -> egui::Response {
        let mut val = ctx.extract_value().unwrap().unwrap();
        let res = match val {
            SteelVal::NumV(ref mut f) => ui.add(egui::DragValue::new(f)),
            SteelVal::IntV(ref mut i) => ui.add(egui::DragValue::new(i)),
            _ => ui.add(egui::Label::new("ERR")),
        };
        if res.changed() {
            ctx.update_value(val).unwrap();
            ctx.push_eval();
        }
        res
    }
}
