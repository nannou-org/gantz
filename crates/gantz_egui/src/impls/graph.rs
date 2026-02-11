use crate::{Cmd, NodeCtx, NodeUi, Registry, widget::node_inspector};

impl<N> NodeUi for gantz_core::node::GraphNode<N>
where
    N: gantz_ca::CaHash,
{
    fn name(&self, _: &dyn Registry) -> &str {
        "graph"
    }

    fn ui(
        &mut self,
        ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui::InnerResponse<egui::Response> {
        uictx.framed(|ui| {
            let res = ui.add(egui::Label::new("graph").selectable(false));
            if ui.response().double_clicked() {
                ctx.cmds.push(Cmd::OpenGraph(ctx.path().to_vec()));
            }
            res
        })
    }

    fn inspector_rows(&mut self, _ctx: &mut NodeCtx, body: &mut egui_extras::TableBody) {
        let row_h = node_inspector::table_row_h(body.ui_mut());
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("CA");
            });
            row.col(|ui| {
                let ca = gantz_ca::graph_addr(&self.graph);
                let ca_string = format!("{}", ca.display_short());
                ui.add(egui::Label::new(egui::RichText::new(ca_string).monospace()));
            });
        });
    }
}
