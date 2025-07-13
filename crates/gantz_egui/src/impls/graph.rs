use crate::{Cmd, NodeCtx, NodeUi};

impl<N> NodeUi for gantz_core::node::GraphNode<N> {
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
}
