use crate::{NodeCtx, NodeUi, NodeUiResponse, Registry, SocketDoc, SocketKind};
use steel::SteelVal;

impl NodeUi for gantz_std::number::Number {
    fn name(&self, _: &dyn Registry) -> &str {
        "number"
    }

    fn description(&self) -> Option<&'static str> {
        Some("A numeric value")
    }

    fn ui(&mut self, mut ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        // The numeric value lives in VM runtime state, not the node weight, so
        // editing it does NOT change the graph's content address - we only
        // queue an evaluation, never mark `changed`.
        let mut do_eval = false;
        let framed = uictx.framed(|ui, _sockets| {
            let mut val = ctx.extract_value().unwrap().unwrap();
            let res = match val {
                SteelVal::NumV(ref mut f) => ui.add(egui::DragValue::new(f)),
                SteelVal::IntV(ref mut i) => ui.add(egui::DragValue::new(i)),
                _ => ui.add(egui::Label::new("ERR")),
            };
            if res.changed() {
                ctx.update_value(val).unwrap();
                do_eval = true;
            }
            res
        });
        let mut resp = NodeUiResponse::new(framed);
        if do_eval {
            resp.push_eval(ctx.path(), 1);
        }
        resp
    }

    fn socket_doc(&self, _: &dyn Registry, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        Some(match kind {
            SocketKind::Input => SocketDoc::ty("number")
                .with_description("new value to store; if unconnected the stored value is reused"),
            SocketKind::Output => {
                SocketDoc::ty("number").with_description("the current stored value")
            }
        })
    }
}
