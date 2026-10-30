//! A node that triggers push evaluation every update, outputting delta time.
//!
//! All `UpdateBang` nodes in a graph are combined into a single multi-source
//! entrypoint by [`entrypoints()`]. The [`drive_update_bangs`] Bevy system
//! drives evaluation, not the node's `ui()` method, so it continues when the
//! graph tab is not visible.
//!
//! This bangs once per update, not once per rendered frame. Under
//! presentation modes like Mailbox, updates can occur more frequently than
//! frames are presented.

use bevy_ecs::prelude::*;
use bevy_egui::egui;
use bevy_gantz::node::{NodeOf, NodePath, NodeState};
use bevy_time::prelude::*;
use gantz_core::compile::EvalSource;
use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_egui::node::DynNode;
use gantz_egui::widget::kit;
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use steel::SteelVal;

/// A node that drives continuous evaluation every update.
///
/// Outputs the update's delta time in seconds as `f64`. This fires once per
/// update, which may be more frequent than rendered frames under presentation
/// modes like Mailbox.
#[derive(
    Clone, Component, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize, NodeTag,
)]
pub struct UpdateBang;

impl gantz_core::Node for UpdateBang {
    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        true
    }

    fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
        node::parse_expr("(begin state)")
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        let path = ctx.path();
        node::state::init_value_if_absent(ctx.vm(), path, || SteelVal::NumV(0.0)).unwrap()
    }
}

impl gantz_egui::NodeUi for UpdateBang {
    fn name(&self, _: &gantz_egui::Env<'_>) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("update!")
    }

    fn description(&self) -> Option<&'static str> {
        Some(
            "Bangs once per app update, with the time since the last update in \
             seconds. Updates can outpace rendered frames, as with Mailbox \
             presentation.",
        )
    }

    fn ui(
        &mut self,
        _ctx: gantz_egui::NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> gantz_egui::NodeUiResponse {
        let dot_id = uictx.egui_id().with("dot");
        let framed = uictx.framed(|ui, _sockets| {
            kit::event_label(ui, dot_id, Some("update"), egui::Sense::hover())
        });
        gantz_egui::NodeUiResponse::new(framed)
    }

    fn socket_doc(
        &self,
        _: &gantz_egui::Env<'_>,
        kind: gantz_egui::SocketKind,
        _ix: usize,
    ) -> Option<gantz_egui::SocketDoc> {
        match kind {
            gantz_egui::SocketKind::Output => Some(
                gantz_egui::SocketDoc::ty("number")
                    .with_description("update delta time in seconds, emitted every update"),
            ),
            gantz_egui::SocketKind::Input => None,
        }
    }
}

/// Collect all `UpdateBang` nodes in the graph and return a single multi-source
/// entrypoint covering all of them.
///
/// Returns an empty vec if no `UpdateBang` nodes are found.
pub fn entrypoints(
    get_node: node::GetNode<'_>,
    graph: &gantz_core::node::graph::Graph<DynNode>,
) -> Vec<gantz_core::compile::Entrypoint> {
    let found = super::find_nodes::<UpdateBang>(get_node, graph);
    if found.is_empty() {
        return vec![];
    }
    let sources = found
        .into_iter()
        .map(|(path, _)| gantz_core::compile::entrypoint::push_source(path, 1));
    vec![gantz_core::compile::entrypoint::from_sources(sources)]
}

/// Drives `update!` nodes every update, independent of GUI visibility.
///
/// Sets the state of each `update!` node to the update's delta time. Then it
/// triggers a single push evaluation per head for all of its `update!` nodes.
pub fn drive_update_bangs(
    time: Res<Time>,
    nodes: Query<(Entity, &NodeOf, &NodePath), With<UpdateBang>>,
    mut state: NodeState,
    mut cmds: Commands,
) {
    let dt = SteelVal::NumV(time.delta_secs_f64());
    let mut heads: HashMap<Entity, Vec<EvalSource>> = HashMap::new();
    for (node, &NodeOf(head), path) in nodes.iter() {
        if let Err(e) = state.set(node, dt.clone()) {
            bevy_log::error!("update! state update failed: {e}");
        }
        let source = gantz_core::compile::entrypoint::push_source(path.0.clone(), 1);
        heads.entry(head).or_default().push(source);
    }
    for (head, sources) in heads {
        cmds.trigger(bevy_gantz::vm::EvalEntryEvent {
            head,
            entrypoint: gantz_core::compile::entrypoint::from_sources(sources),
            // An update represents "now".
            time: None,
        });
    }
}
