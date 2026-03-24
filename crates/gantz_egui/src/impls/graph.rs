use crate::{Cmd, NodeCtx, NodeUi, Registry, widget::node_inspector};
use gantz_core::Node;

impl<N> NodeUi for gantz_core::node::GraphNode<N>
where
    N: gantz_ca::CaHash + Node,
{
    fn name(&self, _: &dyn Registry) -> &str {
        "graph"
    }

    fn ui(
        &mut self,
        ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui_graph::FramedResponse<egui::Response> {
        let registry = ctx.registry();
        let get_node = |ca: &gantz_ca::ContentAddr| registry.node(ca);
        let meta_ctx = gantz_core::node::MetaCtx::new(&get_node);
        let stateful = self.stateful(meta_ctx);
        uictx.framed(|ui, _sockets| {
            let label = if stateful { "graph˚" } else { "graph" };
            let res = ui.add(egui::Label::new(label).selectable(false));
            if ui.response().double_clicked() {
                ctx.cmds.push(Cmd::OpenPath(ctx.path().to_vec()));
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
