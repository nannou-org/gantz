use crate::{Cmd, ContentAddr, NodeCtx, NodeUi, fmt_content_addr, widget::node_inspector};
use serde::{Deserialize, Serialize};
use steel::{SteelVal, parser::ast::ExprKind, steel_vm::engine::Engine};

/// A node abstraction composed from a graph of other nodes.
///
/// A thin wrapper around [`gantz_core::node::Graph`] that allows holding it
/// via an `Arc`, giving the graph a unique name and provides the content
/// address pre-computed to avoid hashing the graph on every update.
///
/// Similar to [`gantz_core::node::GraphNode`], but with a precalculated CA and
/// optional name.
#[derive(Clone, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct NamedGraph {
    name: String,
    ca: ContentAddr,
}

/// The set of node name and content address lookup methods required by the
/// environment for `NamedGraph` nodes.
pub trait GraphRegistry {
    /// The node type of the registered graphs.
    type Node;
    /// Given the content address of a graph, return a reference to the
    /// associated graph.
    fn graph(&self, ca: ContentAddr) -> Option<&gantz_core::node::graph::Graph<Self::Node>>;
}

impl NamedGraph {
    /// Construct a `NamedGraph` node.
    pub fn new(name: String, ca: ContentAddr) -> Self {
        Self { name, ca }
    }
}

impl<Env, N> gantz_core::Node<Env> for NamedGraph
where
    Env: GraphRegistry<Node = N>,
    N: gantz_core::Node<Env>,
{
    fn branches(&self, _env: &Env) -> Vec<gantz_core::node::EvalConf> {
        // TODO: generate branches based on inner node branching
        vec![]
    }

    fn expr(&self, ctx: gantz_core::node::ExprCtx<Env>) -> ExprKind {
        let env = ctx.env();
        env.graph(self.ca)
            .map(|g| gantz_core::node::graph::nested_expr(env, g, ctx.path(), ctx.inputs()))
            // FIXME: Check if graph
            .expect("failed to find graph for CA")
    }

    fn n_inputs(&self, env: &Env) -> usize {
        env.graph(self.ca)
            .map(|g| gantz_core::node::graph::inlets(g).count())
            .unwrap_or(0)
    }

    fn n_outputs(&self, env: &Env) -> usize {
        env.graph(self.ca)
            .map(|g| gantz_core::node::graph::outlets(g).count())
            .unwrap_or(0)
    }

    fn stateful(&self) -> bool {
        true
    }

    fn register(&self, path: &[gantz_core::node::Id], vm: &mut Engine) {
        // Register the graph's state map.
        gantz_core::node::state::update_value(vm, path, SteelVal::empty_hashmap())
            .expect("failed to register graph hashmap");
    }

    fn visit(
        &self,
        ctx: gantz_core::visit::Ctx<Env>,
        visitor: &mut dyn gantz_core::node::Visitor<Env>,
    ) {
        let env = ctx.env();
        if let Some(g) = env.graph(self.ca) {
            gantz_core::graph::visit(env, g, ctx.path(), visitor);
        }
    }
}

impl<Env> NodeUi<Env> for NamedGraph {
    fn name(&self, _: &Env) -> &str {
        self.name.as_str()
    }

    fn ui(&mut self, ctx: NodeCtx<Env>, ui: &mut egui::Ui) -> egui::Response {
        // FIXME: Check if the graph actually exists for the internal CA, give
        // feedback if it doesn't.
        let res = ui.add(egui::Label::new(&self.name).selectable(false));
        if ui.response().double_clicked() {
            ctx.cmds
                .push(Cmd::OpenNamedGraph(self.name.clone(), self.ca));
        }
        res
    }

    fn inspector_rows(&mut self, _ctx: &NodeCtx<Env>, body: &mut egui_extras::TableBody) {
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
