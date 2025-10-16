use crate::{fmt_content_addr, graph_content_addr, widget::node_inspector, Cmd, NodeCtx, NodeUi};
use std::hash::Hash;

impl<N> NodeUi for gantz_core::node::GraphNode<N>
where
    N: Hash,
{
    fn name(&self) -> &str {
        "graph"
    }

    fn ui(&mut self, ctx: NodeCtx, ui: &mut egui::Ui) -> egui::Response {
        let res = ui.add(egui::Label::new("graph").selectable(false));
        if ui.response().double_clicked() {
            ctx.cmds.push(Cmd::OpenGraph(ctx.path().to_vec()));
        }
        res
    }

    fn inspector_rows(&mut self, _ctx: &NodeCtx, body: &mut egui_extras::TableBody) {
        let row_h = node_inspector::table_row_h(body.ui_mut());
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("CA");
            });
            row.col(|ui| {
                let ca = graph_content_addr(self);
                let ca_string = fmt_content_addr(ca);
                ui.add(egui::Label::new(egui::RichText::new(ca_string).monospace()));
            });
        });
    }
}
