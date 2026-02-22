//! Bevy plugin for gantz - an environment for creative systems.
//!
//! This crate provides core Bevy integration for gantz. For egui-based UI,
//! see the `bevy_gantz_egui` crate.

pub mod builtin;
pub mod debounced_input;
pub mod head;
pub mod reg;
pub mod storage;
pub mod vm;

use bevy_app::{App, Plugin, Update};
pub use builtin::{BuiltinNodes, Builtins};
use gantz_core::Node;
pub use head::{
    CompiledModule, FocusedHead, HeadRef, HeadTabOrder, HeadVms, OpenHead, OpenHeadData,
    OpenHeadDataReadOnly, WorkingGraph,
};
pub use reg::{Registry, lookup_node, timestamp};
pub use vm::{EvalCompleted, EvalEvent, EvalKind};

/// Plugin providing core gantz functionality.
///
/// Generic over `N`, the node type used in graphs.
///
/// This plugin:
/// - Initializes core resources (Registry, HeadVms, etc.)
/// - Registers event observers for head operations
/// - Registers the eval event observer
/// - Handles VM initialization for opened/replaced heads
/// - Detects graph changes and recompiles VMs
///
/// Apps should also:
/// - Insert a `BuiltinNodes<N>` resource with their builtin nodes
/// - Add `GantzEguiPlugin` for egui integration (Views, GraphViews, etc.)
pub struct GantzPlugin<N>(std::marker::PhantomData<N>);

impl<N> Default for GantzPlugin<N> {
    fn default() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<N> Plugin for GantzPlugin<N>
where
    N: 'static + Node + Clone + gantz_ca::CaHash + Send + Sync,
{
    fn build(&self, app: &mut App) {
        app.init_resource::<FocusedHead>()
            .init_resource::<HeadTabOrder>()
            .init_resource::<Registry<N>>()
            .init_non_send_resource::<HeadVms>()
            // Register head event handlers.
            .add_observer(head::on_open::<N>)
            .add_observer(head::on_replace::<N>)
            .add_observer(head::on_close::<N>)
            .add_observer(head::on_branch::<N>)
            .add_observer(head::on_move_branch::<N>)
            // Register eval event handler.
            .add_observer(vm::on_eval)
            // VM init observers.
            .add_observer(vm::on_head_opened::<N>)
            .add_observer(vm::on_head_replaced::<N>)
            // Graph recompilation system.
            .add_systems(Update, vm::update::<N>);
    }
}

/// Clone a graph.
pub fn clone_graph<N: Clone>(
    graph: &gantz_core::node::graph::Graph<N>,
) -> gantz_core::node::graph::Graph<N> {
    graph.map(|_, n| n.clone(), |_, e| *e)
}
