//! The `bind` node holds a path to a node in its graph.
//!
//! Element base graphs such as `dialer` and `value` take the path on their
//! `bind` inlet and `ref-gui` takes an instance id, so a GUI built from
//! nodes never carries a hand-typed index. The path is a node weight, so
//! [`crate::ops::remove_nodes`] re-targets it when a removal swaps node
//! indices, and empties it when the target itself is removed. An empty path
//! fails to compile, which flags the node with the ordinary diagnostic glow.

use crate::{Env, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind};
use gantz_ca::DataGraph;
use gantz_core::node::{self, ExprCtx, ExprError, ExprResult, MetaCtx};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// A reference to a node in the same graph, by path.
///
/// The output is the path as a quoted list, for example `'(1)`. The path is
/// relative to the defining graph, matching the `bind` attribute of
/// `gantz_ui` elements.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize, NodeTag)]
pub struct Bind {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    path: Vec<node::Id>,
}

impl Bind {
    /// A bind targeting `path`.
    pub fn new(path: Vec<node::Id>) -> Self {
        Bind { path }
    }

    /// The target path. Empty means no target.
    pub fn path(&self) -> &[node::Id] {
        &self.path
    }

    /// Set the target path. This affects the content address.
    pub fn set_path(&mut self, path: Vec<node::Id>) {
        self.path = path;
    }
}

impl gantz_core::Node for Bind {
    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
        if self.path.is_empty() {
            return Err(ExprError::custom(
                "bind has no target node. Pick its target in the node inspector. \
                 The path is also emptied when the bound node is removed",
            ));
        }
        node::parse_expr(&format!("'({})", path_text(&self.path)))
    }
}

impl NodeUi for Bind {
    fn name(&self, _: &Env<'_>) -> Cow<'_, str> {
        "bind".into()
    }

    fn description(&self) -> Option<&'static str> {
        Some(
            "A path to a node in this graph, for an element's bind inlet or ref-gui. \
             See the demo-gui graph",
        )
    }

    fn ui(&mut self, _ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let framed = uictx.framed(|ui, _sockets| {
            let text = match self.path.is_empty() {
                true => "bind ?".to_string(),
                false => format!("bind {}", path_text(&self.path)),
            };
            ui.add(egui::Label::new(text).selectable(false))
        });
        NodeUiResponse::new(framed)
    }

    fn inspector_rows(
        &mut self,
        ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        let mut resp = InspectorRowsResponse::default();
        let row_h = crate::widget::node_inspector::table_row_h(body.ui_mut());
        let me = ctx.path().last().copied();
        let targets = targets(ctx.env(), ctx.graph(), me);
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("target")
                    .on_hover_text("a node with state, or one a button can push");
            });
            row.col(|ui| {
                // A path that no longer names an eligible node stays visible
                // as `ix (?)` rather than going blank.
                let selected = match self.path.as_slice() {
                    [] => "none".to_string(),
                    [ix] => targets
                        .iter()
                        .find(|(id, _)| id == ix)
                        .map(|(id, name)| format!("{id} ({name})"))
                        .unwrap_or_else(|| format!("{ix} (?)")),
                    path => path_text(path),
                };
                // Every node in the inspector pane shares one child ui id, so
                // the popup id must carry the node path or all bind combos
                // open and close each other.
                egui::ComboBox::from_id_salt(("bind-target", ctx.path()))
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for (ix, name) in &targets {
                            let is_current = self.path.as_slice() == [*ix];
                            let label = format!("{ix} ({name})");
                            if ui.selectable_label(is_current, label).clicked() && !is_current {
                                self.path = vec![*ix];
                                resp.mark_changed();
                            }
                        }
                    });
            });
        });
        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        match kind {
            SocketKind::Output => {
                Some(SocketDoc::ty("path").with_description("the target node's path as a list"))
            }
            SocketKind::Input => None,
        }
    }
}

/// The path as space-separated indices.
fn path_text(path: &[node::Id]) -> String {
    path.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The nodes a bind can target, as `(id, name)` in index order. A node is
/// eligible when it holds state an element can bind, or accepts a push eval
/// a button can fire. `me` is left out. A node the codec cannot reify is
/// left out too, since nothing could render against it.
fn targets(env: &Env<'_>, graph: &DataGraph, me: Option<node::Id>) -> Vec<(node::Id, String)> {
    use gantz_core::Node;
    use petgraph::visit::{IntoNodeReferences, NodeRef};
    let get_node = |ca: &gantz_ca::ContentAddr| env.node(ca);
    let meta = MetaCtx::new(&get_node);
    graph
        .node_references()
        .filter(|n| Some(n.id().index()) != me)
        .filter_map(|n| {
            let inst = env.codec.reify_ui(n.weight()).ok()?;
            let eligible = inst.node.stateful(meta) || !inst.node.push_eval(meta).is_empty();
            eligible.then(|| (n.id().index(), inst.node.name(env).into_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_core::Node;

    fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
        None
    }

    #[test]
    fn expr_is_the_quoted_path() {
        let outputs = node::Conns::connected(1).unwrap();
        let ctx = ExprCtx::new(&no_lookup, &[0], &[], &outputs);
        let expr = Bind::new(vec![1, 2]).expr(ctx).unwrap();
        assert_eq!(expr.to_string(), "(quote (1 2))");
    }

    #[test]
    fn empty_path_fails_to_compile() {
        let outputs = node::Conns::connected(1).unwrap();
        let ctx = ExprCtx::new(&no_lookup, &[0], &[], &outputs);
        assert!(Bind::default().expr(ctx).is_err());
    }

    #[test]
    fn path_text_joins_segments() {
        assert_eq!(path_text(&[1, 2]), "1 2");
        assert_eq!(path_text(&[]), "");
    }

    // Only nodes with state or a push entrypoint are targets. The bind
    // itself is left out.
    #[test]
    fn targets_are_stateful_or_pushable() {
        let registry = gantz_ca::Registry::default();
        let graphs = gantz_core::data::ReifiedGraphs::new();
        let builtins = gantz_core::Builtins::default();
        let instances = crate::node::UiBuiltins::default();
        let codec = crate::test_node::codec();
        let env = crate::Env {
            registry: &registry,
            builtins: &builtins,
            codec: &codec,
            graphs: &graphs,
            instances: &instances,
        };
        let erase = |n: &gantz_core::node::Expr| gantz_core::data::erase_node_typed(n).unwrap();
        let mut graph = DataGraph::default();
        graph.add_node(erase(&gantz_core::node::Expr::new("(+ 1 2)").unwrap())); // 0
        graph.add_node(erase(
            &gantz_core::node::Expr::new("(begin (set! state 1) state)").unwrap(),
        )); // 1
        graph.add_node(gantz_core::data::erase_node_typed(&gantz_std::Bang).unwrap()); // 2
        graph.add_node(gantz_core::data::erase_node_typed(&Bind::default()).unwrap()); // 3

        let ids = |me| {
            targets(&env, &graph, me)
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(Some(3)), vec![1, 2]);
        assert_eq!(ids(Some(1)), vec![2]);
    }
}
