use crate::{fmt_content_addr, widget::node_inspector, Cmd, ContentAddr, NodeCtx, NodeUi};
use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};
use steel::{SteelVal, parser::ast::ExprKind, steel_vm::engine::Engine};

/// A node abstraction composed from a graph of other nodes.
///
/// A thin wrapper around [`gantz_core::node::Graph`] that allows holding it
/// via an `Arc`, giving the graph a unique name and provides the content
/// address pre-computed to avoid hashing the graph on every update.
///
/// Similar to [`gantz_core::node::GraphNode`], but with a precalculated CA and
/// optional name.
pub struct NamedGraph<N> {
    name: String,
    ca: ContentAddr,
    // FIXME: can't include this and be `Serialize`/`Deserialize`.
    // Need a way to pass through the node map / registry during `expr`.
    graph: Arc<gantz_core::node::graph::Graph<N>>,
}

impl<N> Hash for NamedGraph<N>
where
    N: Hash,
{
    fn hash<H>(&self, hasher: &mut H)
    where
        H: Hasher,
    {
        gantz_core::graph::hash(&*self.graph, hasher);
    }
}

impl<N> gantz_core::Node for NamedGraph<N>
where
    N: gantz_core::Node,
{
    fn branches(&self) -> Vec<gantz_core::node::EvalConf> {
        // TODO: generate branches based on inner node branching
        vec![]
    }

    fn expr(&self, ctx: gantz_core::node::ExprCtx) -> ExprKind {
        gantz_core::node::graph::nested_expr(&*self.graph, ctx.path(), ctx.inputs())
    }

    fn n_inputs(&self) -> usize {
        gantz_core::node::graph::inlets(&*self.graph).count()
    }

    fn n_outputs(&self) -> usize {
        gantz_core::node::graph::outlets(&*self.graph).count()
    }

    fn stateful(&self) -> bool {
        true
    }

    fn register(&self, path: &[gantz_core::node::Id], vm: &mut Engine) {
        // Register the graph's state map.
        gantz_core::node::state::update_value(vm, path, SteelVal::empty_hashmap())
            .expect("failed to register graph hashmap");
    }

    fn visit(&self, ctx: gantz_core::visit::Ctx, visitor: &mut dyn gantz_core::node::Visitor) {
        gantz_core::graph::visit(&*self.graph, ctx.path(), visitor);
    }
}

impl<N> NodeUi for NamedGraph<N>
where
    N: Hash,
{
    fn name(&self) -> &str {
        self.name.as_str()
    }

    fn ui(&mut self, ctx: NodeCtx, ui: &mut egui::Ui) -> egui::Response {
        let res = ui.add(egui::Label::new(&self.name).selectable(false));
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
                let ca_string = fmt_content_addr(self.ca);
                ui.add(egui::Label::new(egui::RichText::new(ca_string).monospace()));
            });
        });
    }
}
