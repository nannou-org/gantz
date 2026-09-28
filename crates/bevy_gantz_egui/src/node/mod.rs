pub use await_::Await;
pub use gui_refresh::GuiMarkersDirty;
pub use sleep::Sleep;
pub use tick_bang::{Interval, TickBang};
pub use update_bang::UpdateBang;

pub mod await_;
pub mod gui_refresh;
pub mod sleep;
pub mod tick_bang;
pub mod update_bang;

/// Builtin specs for the bevy node set.
pub fn builtins() -> Vec<gantz_core::Builtin> {
    use gantz_core::Builtin;
    vec![
        Builtin::new("await", &Await),
        Builtin::new("sleep", &Sleep::default()),
        Builtin::new("tick!", &TickBang::default()),
        Builtin::new("update!", &UpdateBang),
    ]
}

/// Every entrypoint `GantzEguiPlugin` compiles for a graph: the push and pull
/// sources, the `update!` and `tick!` sources, and the root `main!` sources.
pub fn entrypoints(
    get_node: gantz_core::node::GetNode<'_>,
    graph: &gantz_core::node::graph::Graph<gantz_egui::node::DynNode>,
) -> Vec<gantz_core::compile::Entrypoint> {
    let mut eps = gantz_core::compile::push_pull_entrypoints(get_node, graph);
    eps.extend(update_bang::entrypoints(get_node, graph));
    eps.extend(tick_bang::entrypoints(get_node, graph));
    eps.extend(gantz_io::main_bang::entrypoint(graph));
    eps
}
