//! The `Gui` marker node: declares a graph's GUI from within the graph.
//!
//! Just as [`Inlet`][gantz_core::node::graph::Inlet]/[`Outlet`][gantz_core::node::graph::Outlet]
//! markers declare a graph's sockets, a [`Gui`] marker declares its GUI: the
//! tree pull-evaluated into the marker is stored in the marker's state slot,
//! where the host reads it and renders it via the `ui_tree` interpreter. One
//! marker per [`GuiRole`] (a duplicate role is resolved as first-in-index-order
//! wins).

use crate::widget::node_inspector::{self, radio_option};
use crate::{Env, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind};
use gantz_core::node::{self, EvalConf, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// The ext key under which a ref stores its per-instance GUI overrides.
pub const GUI_REF_EXT_KEY: &str = "gantz.gui";

/// The `NodeUi` surface a marker's tree presents.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GuiRole {
    /// The in-graph node form (`NodeUi::ui`).
    #[default]
    Body,
    /// The detached pane (`NodeUi::view_ui`).
    View,
    /// Appended after the inspector's default table (`NodeUi::inspector_ui`).
    Inspector,
    /// Condensed body variant for dense patching.
    Compact,
}

/// How instances present the graph's body GUI by default.
///
/// Meaningful on the [`GuiRole::Body`] marker; instances may override it via
/// [`GuiRefExt`].
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GuiDisplay {
    /// Render the full body marker tree.
    #[default]
    Full,
    /// Render the compact marker tree (falls back to a label).
    Compact,
    /// Render the name label only.
    Label,
}

/// A marker node declaring the tree wired into it as this graph's GUI for
/// `role`.
///
/// Stateful with a single pull-evaluated input: the pull stores the incoming
/// tree in the marker's state slot, where the host reads it each frame.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize, NodeTag)]
pub struct Gui {
    /// The surface this marker's tree presents.
    #[serde(default)]
    pub role: GuiRole,
    /// The default display mode instances use for the body GUI.
    #[serde(default)]
    pub display: GuiDisplay,
}

/// Per-instance GUI overrides, stored in a ref's ext map under
/// [`GUI_REF_EXT_KEY`].
///
/// Absent means "follow the definition default" (the body marker's
/// [`display`][Gui::display]); present means the user chose an explicit
/// display for this instance, even if it matches the definition default.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct GuiRefExt {
    /// The display mode this instance renders the referenced GUI with.
    #[serde(default)]
    pub display: GuiDisplay,
}

impl GuiRole {
    /// All roles, in inspector/palette order.
    pub const ALL: [Self; 4] = [Self::Body, Self::View, Self::Inspector, Self::Compact];

    /// The lowercase name used in labels, sugar and the wire format.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::View => "view",
            Self::Inspector => "inspector",
            Self::Compact => "compact",
        }
    }

    /// The role for its lowercase name.
    pub fn from_str(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_str() == s)
    }
}

impl GuiDisplay {
    /// All display modes, in inspector/palette order.
    pub const ALL: [Self; 3] = [Self::Full, Self::Compact, Self::Label];

    /// The lowercase name used in labels, sugar and the wire format.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Compact => "compact",
            Self::Label => "label",
        }
    }

    /// The display mode for its lowercase name.
    pub fn from_str(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.as_str() == s)
    }
}

impl gantz_core::Node for Gui {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        true
    }

    fn pull_eval(&self, _ctx: MetaCtx) -> Vec<EvalConf> {
        vec![EvalConf::All]
    }

    fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult {
        let expr = match ctx.inputs().get(0) {
            Some(Some(val)) => format!("(begin (set! state {val}) state)"),
            _ => "(begin state)".to_string(),
        };
        node::parse_expr(&expr)
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        let path = ctx.path();
        node::state::init_value_if_absent(ctx.vm(), path, || steel::SteelVal::Void).unwrap()
    }
}

impl NodeUi for Gui {
    fn name(&self, _: &Env<'_>) -> Cow<'_, str> {
        "gui".into()
    }

    fn description(&self) -> Option<&'static str> {
        Some("Declares the wired tree as this graph's GUI for a role")
    }

    fn ui(&mut self, _ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let framed = uictx.framed(|ui, _sockets| {
            let text = format!("gui[{}]", self.role.as_str());
            ui.add(egui::Label::new(text).selectable(false))
        });
        NodeUiResponse::new(framed)
    }

    fn inspector_rows(
        &mut self,
        _ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        let mut changed = false;
        let row_h = node_inspector::table_row_h(body.ui_mut());

        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("role");
            });
            row.col(|ui| {
                ui.horizontal(|ui| {
                    changed |= radio_option(
                        ui,
                        &mut self.role,
                        GuiRole::Body,
                        "body",
                        "the in-graph node form",
                    );
                    changed |= radio_option(
                        ui,
                        &mut self.role,
                        GuiRole::View,
                        "view",
                        "the detached view pane",
                    );
                    changed |= radio_option(
                        ui,
                        &mut self.role,
                        GuiRole::Inspector,
                        "inspector",
                        "appended after the inspector table",
                    );
                    changed |= radio_option(
                        ui,
                        &mut self.role,
                        GuiRole::Compact,
                        "compact",
                        "condensed body for dense patching",
                    );
                });
            });
        });

        if self.role == GuiRole::Body {
            body.row(row_h, |mut row| {
                row.col(|ui| {
                    ui.label("display");
                });
                row.col(|ui| {
                    ui.horizontal(|ui| {
                        changed |= radio_option(
                            ui,
                            &mut self.display,
                            GuiDisplay::Full,
                            "full",
                            "instances render the full body tree",
                        );
                        changed |= radio_option(
                            ui,
                            &mut self.display,
                            GuiDisplay::Compact,
                            "compact",
                            "instances render the compact tree",
                        );
                        changed |= radio_option(
                            ui,
                            &mut self.display,
                            GuiDisplay::Label,
                            "label",
                            "instances render the name label",
                        );
                    });
                });
            });
        }

        let mut resp = InspectorRowsResponse::default();
        resp.set_changed(changed);
        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        match kind {
            SocketKind::Input => Some(
                SocketDoc::ty("ui tree")
                    .with_description("stored and presented as this graph's GUI for the role"),
            ),
            SocketKind::Output => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_core::node::{Node, WithPushEval};
    use gantz_core::{
        Edge, ROOT_STATE,
        compile::{EvalKind, entry_fn_name, entrypoint, push_pull_entrypoints},
    };
    use steel::SteelVal;
    use steel::steel_vm::engine::Engine;

    // A node lookup is unnecessary for these self-contained graphs.
    fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
        None
    }

    // Compile `g`, init a base VM with node state, and load the module.
    fn vm_for(g: &petgraph::graph::DiGraph<Box<dyn Node>, Edge>) -> Engine {
        let eps = push_pull_entrypoints(&no_lookup, g);
        let module = gantz_core::compile::module(&no_lookup, g, &eps, &Default::default()).unwrap();
        let mut vm = Engine::new_base();
        vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
        gantz_core::graph::register(&no_lookup, g, &[], &mut vm);
        for f in module {
            vm.run(format!("{f}")).unwrap();
        }
        vm
    }

    // Fire the pull entrypoint of the marker at `path`.
    fn fire_pull(vm: &mut Engine, path: Vec<usize>) {
        let ep = entrypoint::pull(path, 1);
        let fn_name = entry_fn_name(&ep.id());
        vm.call_function_by_name_with_args(&fn_name, vec![])
            .unwrap();
    }

    // Fire the push entrypoint of node `ix`.
    fn fire_push(vm: &mut Engine, g: &petgraph::graph::DiGraph<Box<dyn Node>, Edge>, ix: usize) {
        let ctx = node::MetaCtx::new(&no_lookup);
        let outs = g[petgraph::graph::NodeIndex::new(ix)].n_outputs(ctx) as u8;
        let ep = entrypoint::push(vec![ix], outs);
        let fn_name = entry_fn_name(&ep.id());
        vm.call_function_by_name_with_args(&fn_name, vec![])
            .unwrap();
    }

    // The marker's stored state as a list, panicking on any other shape.
    fn list_state(vm: &Engine, path: &[usize]) -> Vec<SteelVal> {
        match node::state::extract_value(vm, path).unwrap().unwrap() {
            SteelVal::ListV(list) => list.iter().cloned().collect(),
            other => panic!("expected list state, got {other:?}"),
        }
    }

    // Build `expr -> gui`, returning the graph and the two node indices.
    fn graph_with(
        src: Box<dyn Node>,
        gui: Gui,
    ) -> (petgraph::graph::DiGraph<Box<dyn Node>, Edge>, usize, usize) {
        let mut g = petgraph::graph::DiGraph::new();
        let s = g.add_node(src);
        let m = g.add_node(Box::new(gui) as Box<dyn Node>);
        g.add_edge(s, m, Edge::from((0, 0)));
        (g, s.index(), m.index())
    }

    // A pull at the marker evaluates the upstream tree and stores it.
    #[test]
    fn pull_stores_connected_tree() {
        let src = gantz_core::node::expr("'(col)").unwrap();
        let (g, _s, m) = graph_with(Box::new(src) as Box<dyn Node>, Gui::default());
        let mut vm = vm_for(&g);
        fire_pull(&mut vm, vec![m]);
        assert_eq!(list_state(&vm, &[m]).len(), 1);
    }

    // A push through the marker stores the pushed tree.
    #[test]
    fn push_through_stores_tree() {
        let src = gantz_core::node::expr("'(row (sep))")
            .unwrap()
            .with_push_eval();
        let (g, s, m) = graph_with(Box::new(src) as Box<dyn Node>, Gui::default());
        let mut vm = vm_for(&g);
        fire_push(&mut vm, &g, s);
        assert_eq!(list_state(&vm, &[m]).len(), 2);
    }

    // An unconnected marker's pull leaves its registered Void state.
    #[test]
    fn unconnected_pull_keeps_void() {
        let mut g = petgraph::graph::DiGraph::<Box<dyn Node>, Edge>::new();
        let m = g
            .add_node(Box::new(Gui::default()) as Box<dyn Node>)
            .index();
        let mut vm = vm_for(&g);
        fire_pull(&mut vm, vec![m]);
        let val = node::state::extract_value(&vm, &[m]).unwrap().unwrap();
        assert_eq!(val, SteelVal::Void);
    }

    // A marker nested in an inner graph gets its own per-instance pull
    // entrypoint (collected through the nesting), and firing it stores the
    // inner tree at the nested path. Gui is the first 0-output stateful node,
    // so this also pins the compiled form.
    #[test]
    fn nested_marker_pull_entrypoint_fires() {
        let mut inner = gantz_core::node::graph::Graph::<Box<dyn Node>>::default();
        let src = gantz_core::node::expr("'(col)").unwrap();
        let s = inner.add_node(Box::new(src) as Box<dyn Node>);
        let gui = inner.add_node(Box::new(Gui::default()) as Box<dyn Node>);
        let m = gui.index();
        inner.add_edge(s, gui, Edge::from((0, 0)));

        let mut outer = petgraph::graph::DiGraph::<Box<dyn Node>, Edge>::new();
        let n = outer.add_node(Box::new(inner) as Box<dyn Node>).index();

        // The collector emits the nested marker's singleton pull entrypoint.
        let eps = push_pull_entrypoints(&no_lookup, &outer);
        let expected = entrypoint::pull(vec![n, m], 1);
        assert!(
            eps.iter()
                .any(|ep| { ep == &expected && ep.0.iter().all(|s| s.kind == EvalKind::Pull) }),
            "expected a singleton pull entrypoint at [{n}, {m}], got {eps:?}"
        );

        let mut vm = vm_for(&outer);
        fire_pull(&mut vm, vec![n, m]);
        assert_eq!(list_state(&vm, &[n, m]).len(), 1);
    }
}
