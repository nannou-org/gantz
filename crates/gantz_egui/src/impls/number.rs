use crate::{NodeCtx, NodeUi, Registry, SocketDoc};
use steel::SteelVal;

impl NodeUi for gantz_std::number::Number {
    fn name(&self, _: &dyn Registry) -> &str {
        "number"
    }

    fn ui(
        &mut self,
        mut ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui_graph::FramedResponse<egui::Response> {
        uictx.framed(|ui, _sockets| {
            let mut val = ctx.extract_value().unwrap().unwrap();
            let res = match val {
                SteelVal::NumV(ref mut f) => ui.add(egui::DragValue::new(f)),
                SteelVal::IntV(ref mut i) => ui.add(egui::DragValue::new(i)),
                _ => ui.add(egui::Label::new("ERR")),
            };
            if res.changed() {
                ctx.update_value(val).unwrap();
                ctx.push_eval(1);
            }
            res
        })
    }

    fn input_doc(&self, _: &dyn Registry, _ix: usize) -> Option<SocketDoc> {
        Some(
            SocketDoc::ty("number")
                .with_description("New value to store; if unconnected the stored value is reused"),
        )
    }

    fn output_doc(&self, _: &dyn Registry, _ix: usize) -> Option<SocketDoc> {
        Some(SocketDoc::ty("number").with_description("The current stored value"))
    }
}
