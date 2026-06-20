//! Bevy plugin for gantz - an environment for creative systems.
//!
//! This crate provides core Bevy integration for gantz. For egui-based UI,
//! see the `bevy_gantz_egui` crate.
//!
//! # Events vs Messages
//!
//! Observer events (`Event` + `On<T>`) are used for discrete, low-frequency
//! intents and hooks where immediate, possibly-cascading handling matters.
//! These come in two layers:
//!
//! - *Request* events ask for an operation: [`head::OpenEvent`],
//!   [`head::CloseEvent`], [`head::ReplaceEvent`], [`head::BranchHeadEvent`],
//!   [`head::MoveBranchEvent`], [`vm::EvalEntryEvent`].
//! - *Hook* events announce that one happened, decoupling this crate from
//!   downstream UI crates: [`head::OpenedEvent`], [`head::ClosedEvent`],
//!   [`head::ChangedEvent`], [`head::BranchedHeadEvent`],
//!   [`head::CommittedEvent`], [`vm::EvalEntryComplete`].
//!
//! Buffered messages (`Message` + `MessageReader`) are reserved for
//! per-frame streams consumed by polling systems -
//! [`debounced_input::DebouncedInputEvent`] is the one case.

pub mod builtin;
pub mod debounced_input;
pub mod head;
pub mod reg;
pub mod storage;
pub mod vm;

use bevy_app::{App, Plugin, Update};
use bevy_ecs::prelude::{IntoScheduleConfigs, SystemSet};
pub use builtin::{BuiltinNodes, Builtins};
use gantz_core::Node;
pub use head::{
    FocusedHead, HeadRef, HeadTabOrder, HeadVms, OpenHead, OpenHeadData, OpenHeadDataReadOnly,
    WorkingGraph,
};
pub use reg::{Registry, lookup_node, timestamp};
pub use vm::{CompileConfig, CompiledInputs, EntrypointFns, EvalEntryComplete, EvalEntryEvent};

/// The system set in which [`vm::sync`] runs (in the `Update` schedule).
///
/// Systems that evaluate head VMs each frame should run `.after(VmSet)` so
/// they never observe the gap between a head pointing at a new graph and its
/// VM being (re)initialized.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, SystemSet)]
pub struct VmSet;

/// Plugin providing core gantz functionality.
///
/// Generic over `N`, the node type used in graphs.
///
/// This plugin:
/// - Initializes core resources (Registry, HeadVms, etc.)
/// - Registers event observers for head operations
/// - Registers the eval event observer
/// - Keeps head VMs in sync with their compile inputs via [`vm::sync`]
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
        app.insert_resource(vm::EntrypointFns::<N>(vec![Box::new(|get_node, graph| {
            gantz_core::compile::push_pull_entrypoints(get_node, graph)
        })]))
        .init_resource::<FocusedHead>()
        .init_resource::<HeadTabOrder>()
        .init_resource::<Registry<N>>()
        .init_resource::<vm::CompileConfig>()
        .init_non_send::<HeadVms>()
        // Register head event handlers.
        .add_observer(head::on_open::<N>)
        .add_observer(head::on_replace::<N>)
        .add_observer(head::on_close::<N>)
        .add_observer(head::on_branch_head::<N>)
        .add_observer(head::on_move_branch::<N>)
        // Register eval entry event handler.
        .add_observer(vm::on_eval_entry)
        // Input-addressed VM synchronisation: (re)compiles whenever a head's
        // compile inputs (graph content address + config) change.
        .add_systems(Update, vm::sync::<N>.in_set(VmSet));
    }
}

/// Clone a graph.
pub fn clone_graph<N: Clone>(
    graph: &gantz_core::node::graph::Graph<N>,
) -> gantz_core::node::graph::Graph<N> {
    graph.map(|_, n| n.clone(), |_, e| *e)
}
