use crate::{NodeCtx, NodeUi, Registry};
use steel::SteelVal;

impl NodeUi for gantz_std::number::Number {
    fn name(&self, _: &dyn Registry) -> &str {
        "number"
    }

    fn ui(
        &mut self,
        mut ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui::InnerResponse<egui::Response> {
        uictx.framed(|ui| {
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
        })
    }
}
