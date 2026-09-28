pub use await_::Await;
pub use gui_refresh::GuiMarkersDirty;
pub use sleep::Sleep;
pub use tick_bang::{Interval, TickBang};
pub use update_bang::UpdateBang;

use gantz_core::node::{GetNode, Id, graph::Graph};
use gantz_core::visit;
use gantz_egui::node::DynNode;

pub mod await_;
pub mod gui_refresh;
pub mod sleep;
pub mod tick_bang;
pub mod update_bang;

/// The visitor behind [`find_nodes`].
struct Finder<T>(Vec<(Vec<Id>, T)>);

impl<T: Clone + 'static> visit::TypedVisitor<DynNode> for Finder<T> {
    fn visit_pre(&mut self, ctx: visit::Ctx<'_, '_>, node: &DynNode) {
        let n: &dyn gantz_core::Node = &**node;
        if let Some(found) = (n as &dyn std::any::Any).downcast_ref::<T>() {
            self.0.push((ctx.path().to_vec(), found.clone()));
        }
    }
}

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
    get_node: GetNode<'_>,
    graph: &Graph<DynNode>,
) -> Vec<gantz_core::compile::Entrypoint> {
    let mut eps = gantz_core::compile::push_pull_entrypoints(get_node, graph);
    eps.extend(update_bang::entrypoints(get_node, graph));
    eps.extend(tick_bang::entrypoints(get_node, graph));
    eps.extend(gantz_io::main_bang::entrypoint(graph));
    eps
}

/// Every node of type `T` in the graph tree with its path, found by
/// [`Any`](std::any::Any) downcast of the erased UI node.
pub(crate) fn find_nodes<T: Clone + 'static>(
    get_node: GetNode<'_>,
    graph: &Graph<DynNode>,
) -> Vec<(Vec<Id>, T)> {
    let mut finder = Finder(vec![]);
    gantz_core::graph::visit_typed(get_node, graph, &[], &mut finder);
    finder.0
}
