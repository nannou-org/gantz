pub use await_::Await;
pub use gui_refresh::GuiMarkersDirty;
pub use load_bang::LoadBang;
pub use sleep::Sleep;
pub use tick_bang::{Interval, TickBang};
pub use update_bang::UpdateBang;

use bevy_ecs::prelude::*;
use gantz_core::node::{GetNode, Id, graph::Graph};
use gantz_core::visit;
use gantz_egui::node::DynNode;
use std::collections::HashMap;

pub mod await_;
pub mod gui_refresh;
pub mod load_bang;
pub mod sleep;
pub mod tick_bang;
pub mod update_bang;

/// The visitor behind [`find_nodes`].
struct Finder<T>(Vec<(Vec<Id>, T)>);

/// The paths of one graph tree's driven nodes. See [`DriverIndex`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrivenNodes {
    /// The `update!` nodes.
    pub update: Vec<Vec<Id>>,
    /// The `tick!` nodes with their tick durations.
    pub tick: Vec<(Vec<Id>, f64)>,
    /// The `await` nodes.
    pub awaits: Vec<Vec<Id>>,
}

/// The [`DrivenNodes`] of each open head's committed graph, keyed by the
/// graph's address.
///
/// A graph's address fixes its whole tree, nested graphs included, so an
/// entry never goes stale. [`index_driven_nodes`] walks each graph once and
/// drops the graphs that no open head shows. The drivers then read the paths
/// each frame without a walk.
#[derive(Default, Resource)]
pub struct DriverIndex(HashMap<gantz_ca::GraphAddr, DrivenNodes>);

/// The visitor behind [`driven_nodes`].
struct DrivenFinder(DrivenNodes);

impl DriverIndex {
    /// The driven nodes of the graph at `graph`, if indexed.
    pub fn get(&self, graph: &gantz_ca::GraphAddr) -> Option<&DrivenNodes> {
        self.0.get(graph)
    }
}

impl<T: Clone + 'static> visit::TypedVisitor<DynNode> for Finder<T> {
    fn visit_pre(&mut self, ctx: visit::Ctx<'_, '_>, node: &DynNode) {
        let n: &dyn gantz_core::Node = &**node;
        if let Some(found) = (n as &dyn std::any::Any).downcast_ref::<T>() {
            self.0.push((ctx.path().to_vec(), found.clone()));
        }
    }
}

impl visit::TypedVisitor<DynNode> for DrivenFinder {
    fn visit_pre(&mut self, ctx: visit::Ctx<'_, '_>, node: &DynNode) {
        let n: &dyn gantz_core::Node = &**node;
        let any = n as &dyn std::any::Any;
        if any.is::<UpdateBang>() {
            self.0.update.push(ctx.path().to_vec());
        } else if let Some(tick) = any.downcast_ref::<TickBang>() {
            self.0.tick.push((ctx.path().to_vec(), tick.duration()));
        } else if any.is::<Await>() {
            self.0.awaits.push(ctx.path().to_vec());
        }
    }
}

/// Builtin specs for the bevy node set.
pub fn builtins() -> Vec<gantz_core::Builtin> {
    use gantz_core::Builtin;
    vec![
        Builtin::new("await", &Await),
        Builtin::new("load!", &LoadBang),
        Builtin::new("sleep", &Sleep::default()),
        Builtin::new("tick!", &TickBang::default()),
        Builtin::new("update!", &UpdateBang),
    ]
}

/// Every entrypoint `GantzEguiPlugin` compiles for a graph. These are the push
/// and pull sources, the `load!`, `update!` and `tick!` sources, and the root
/// `main!` sources.
pub fn entrypoints(
    get_node: GetNode<'_>,
    graph: &Graph<DynNode>,
) -> Vec<gantz_core::compile::Entrypoint> {
    let mut eps = gantz_core::compile::push_pull_entrypoints(get_node, graph);
    eps.extend(load_bang::entrypoint(get_node, graph));
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

/// Index the driven nodes of each open head's committed graph. See
/// [`DriverIndex`].
pub fn index_driven_nodes(
    registry: Res<crate::Registry>,
    cache: Res<crate::GraphCache>,
    builtins: Res<crate::BuiltinNodes>,
    heads: Query<&bevy_gantz::head::HeadRef, With<bevy_gantz::head::OpenHead>>,
    mut index: ResMut<DriverIndex>,
) {
    let mut next = HashMap::new();
    for head_ref in heads.iter() {
        let Some(graph_ca) = registry.head_commit(&head_ref.0).map(|c| c.graph) else {
            continue;
        };
        if next.contains_key(&graph_ca) {
            continue;
        }
        let driven = match index.0.remove(&graph_ca) {
            Some(driven) => driven,
            None => {
                // The committed graph equals the working graph, see
                // `bevy_gantz::head::WorkingGraph`.
                let Some(graph) = cache.get(&graph_ca) else {
                    continue;
                };
                let get_node = |ca: &gantz_ca::ContentAddr| {
                    crate::lookup_node(&cache, &builtins.instances, ca)
                };
                driven_nodes(&get_node, graph)
            }
        };
        next.insert(graph_ca, driven);
    }
    index.0 = next;
}

/// The driven nodes in the graph tree.
fn driven_nodes(get_node: GetNode<'_>, graph: &Graph<DynNode>) -> DrivenNodes {
    let mut finder = DrivenFinder(DrivenNodes::default());
    gantz_core::graph::visit_typed(get_node, graph, &[], &mut finder);
    finder.0
}
