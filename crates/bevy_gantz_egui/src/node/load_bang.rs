//! A node that pushes evaluation once when its graph loads, like the
//! `loadbang` of Pd and Max.
//!
//! A graph loads when its head opens in a tab, including restored tabs at
//! startup, and when a [`bevy_gantz::head::ReplaceEvent`] loads it into the
//! focused tab. Edits, undo, redo and merges do not load a graph. All `load!`
//! nodes in the graph tree fire together through the single multi-source
//! entrypoint from [`entrypoint`]. The [`drive_load_bangs`] Bevy system fires
//! it for each head marked with [`bevy_gantz::head::PendingLoad`].

use bevy_ecs::prelude::*;
use bevy_egui::egui;
use bevy_gantz::head::{HeadRef, Module, OpenHead, PendingLoad};
use bevy_gantz::vm::{CompileConfig, CompiledInputs, Inputs};
use gantz_core::node::{self, EvalConf, ExprCtx, ExprResult, MetaCtx};
use gantz_egui::node::DynNode;
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};

/// A node that emits a bang when its graph loads.
///
/// Double-click the node to fire it again by hand.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize, NodeTag)]
pub struct LoadBang;

impl gantz_core::Node for LoadBang {
    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
        node::parse_expr("'()")
    }

    fn push_eval(&self, _ctx: MetaCtx) -> Vec<EvalConf> {
        // The entry fn that a double-click fires.
        vec![EvalConf::All]
    }
}

impl gantz_egui::NodeUi for LoadBang {
    fn name(&self, _: &gantz_egui::Env<'_>) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("load!")
    }

    fn description(&self) -> Option<&'static str> {
        Some(
            "Emits a bang once when the graph loads. A graph loads when it opens \
             in a tab, including at startup, or when it replaces the graph in the \
             focused tab. Edits, undo and redo do not fire it. Double-click to \
             fire it by hand.",
        )
    }

    fn ui(
        &mut self,
        ctx: gantz_egui::NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> gantz_egui::NodeUiResponse {
        let framed =
            uictx.framed(|ui, _sockets| ui.add(egui::Label::new("load!").selectable(false)));
        let mut resp = gantz_egui::NodeUiResponse::new(framed);
        if resp.framed.inner.response.double_clicked() {
            resp.push_eval(ctx.path(), 1);
        }
        resp
    }

    fn socket_doc(
        &self,
        _: &gantz_egui::Env<'_>,
        kind: gantz_egui::SocketKind,
        _ix: usize,
    ) -> Option<gantz_egui::SocketDoc> {
        match kind {
            gantz_egui::SocketKind::Output => Some(
                gantz_egui::SocketDoc::ty("bang")
                    .with_description("empty list '() emitted once when the graph loads"),
            ),
            gantz_egui::SocketKind::Input => None,
        }
    }
}

/// The single push entrypoint that fires every `load!` node in the graph tree
/// together, or `None` if the graph has no `load!` node.
pub fn entrypoint(
    get_node: node::GetNode<'_>,
    graph: &gantz_core::node::graph::Graph<DynNode>,
) -> Option<gantz_core::compile::Entrypoint> {
    let found = super::find_nodes::<LoadBang>(get_node, graph);
    if found.is_empty() {
        return None;
    }
    let sources = found
        .into_iter()
        .map(|(path, _)| gantz_core::compile::entrypoint::push_source(path, 1));
    Some(gantz_core::compile::entrypoint::from_sources(sources))
}

/// Fires the `load!` nodes of each head with a pending load.
///
/// A head's load waits until `vm::sync` has compiled its current graph
/// without error. A graph that fails to compile fires on its first
/// successful compile. Firing removes the [`PendingLoad`] marker.
pub fn drive_load_bangs(
    config: Res<CompileConfig>,
    registry: Res<crate::Registry>,
    cache: Res<crate::GraphCache>,
    builtins: Res<crate::BuiltinNodes>,
    heads: Query<(Entity, &HeadRef, &CompiledInputs, &Module), (With<OpenHead>, With<PendingLoad>)>,
    mut cmds: Commands,
) {
    for (entity, head_ref, compiled_inputs, module) in heads.iter() {
        let Some(graph_ca) = registry.head_commit(&head_ref.0).map(|c| c.graph) else {
            continue;
        };
        let inputs = Inputs {
            graph: graph_ca,
            config: config.0,
        };
        if compiled_inputs.0 != Some(inputs) || module.error.is_some() {
            continue;
        }
        cmds.entity(entity).remove::<PendingLoad>();

        let Some(graph) = cache.get(&graph_ca) else {
            continue;
        };
        let get_node =
            |ca: &gantz_ca::ContentAddr| crate::lookup_node(&cache, &builtins.instances, ca);
        if let Some(entrypoint) = entrypoint(&get_node, graph) {
            cmds.trigger(bevy_gantz::vm::EvalEntryEvent {
                head: entity,
                entrypoint,
                time: None,
            });
        }
    }
}
