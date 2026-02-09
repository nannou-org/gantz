//! Bevy plugin for gantz - an environment for creative systems.
//!
//! This crate provides core Bevy integration for gantz. For egui-based UI,
//! see the `bevy_gantz_egui` crate.

pub mod builtin;
pub mod debounced_input;
pub mod eval;
pub mod head;
pub mod plugin;
pub mod reg;
pub mod storage;
pub mod vm;

pub use builtin::{BuiltinNodes, Builtins};
pub use eval::{EvalEvent, EvalKind, VmExecCompleted};
pub use head::{
    CompiledModule, FocusedHead, HeadRef, HeadTabOrder, HeadVms, OpenHead, OpenHeadData,
    OpenHeadDataReadOnly, WorkingGraph,
};
pub use plugin::GantzPlugin;
pub use reg::{Registry, RegistryRef, timestamp};

/// Clone a graph.
pub fn clone_graph<N: Clone>(
    graph: &gantz_core::node::graph::Graph<N>,
) -> gantz_core::node::graph::Graph<N> {
    graph.map(|_, n| n.clone(), |_, e| *e)
}
