pub mod builtin;
pub mod debounced_input;
pub mod egui;
pub mod eval;
pub mod head;
pub mod plugin;
pub mod reg;
pub mod storage;
pub mod view;
pub mod vm;

pub use builtin::{BuiltinNodes, Builtins};
pub use egui::{
    CreateNodeEvent, GantzEguiPlugin, GuiState, InspectEdgeEvent, PerfGui, PerfVm, TraceCapture,
};
pub use head::{
    BranchCreated, CompiledModule, FocusedHead, GraphViews, HeadAccess, HeadClosed, HeadCommitted,
    HeadGuiState, HeadOpened, HeadRef, HeadReplaced, HeadTabOrder, HeadVms, OpenHead, OpenHeadData,
    OpenHeadDataReadOnly, WorkingGraph,
};
pub use plugin::GantzPlugin;
pub use reg::{Registry, RegistryRef, timestamp};
pub use view::Views;

/// Clone a graph.
pub fn clone_graph<N: Clone>(
    graph: &gantz_core::node::graph::Graph<N>,
) -> gantz_core::node::graph::Graph<N> {
    graph.map(|_, n| n.clone(), |_, e| *e)
}
