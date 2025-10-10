use std::hash::{Hash, Hasher};

use crate::{Cmd, ContentAddr, NodeCtx, NodeUi, widget::node_inspector};

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
                let ca = content_addr(self);
                let ca_string = format!("{ca:#016x}");
                ui.add(egui::Label::new(egui::RichText::new(ca_string).monospace()));
            });
        });
    }
}

/// Produce the content address for a given graph node.
fn content_addr<N>(g: &gantz_core::node::GraphNode<N>) -> ContentAddr
where
    N: Hash,
{
    // TODO: Use a more stable/reproducible hash method.
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    g.hash(&mut hasher);
    hasher.finish()
}
