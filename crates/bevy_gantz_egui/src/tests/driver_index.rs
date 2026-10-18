//! The driven-node index. See `crate::node::DriverIndex`.

use bevy_app::App;
use bevy_gantz::{Registry, head, timestamp};
use gantz_ca::{DataGraph, GraphAddr, Head, Name};
use gantz_core::data::erase_node_typed;
use gantz_egui::node::Inspect;

fn name() -> Name {
    "g".parse().unwrap()
}

/// Commit `graph` to the branch `g` and return the graph's address.
fn commit(app: &mut App, graph: DataGraph) -> GraphAddr {
    let addr = gantz_ca::graph_addr(&graph);
    app.world_mut()
        .resource_mut::<Registry>()
        .commit_graph_to_name(timestamp(), addr, move || graph, &name());
    super::refresh_app_cache(app);
    addr
}

fn graph(n_awaits: usize) -> DataGraph {
    let mut graph = DataGraph::default();
    graph.add_node(erase_node_typed(&Inspect).unwrap());
    for _ in 0..n_awaits {
        graph.add_node(erase_node_typed(&crate::node::Await).unwrap());
    }
    graph
}

/// The indexed `await` paths of `graph`, sorted.
fn awaits(app: &App, graph: &GraphAddr) -> Option<Vec<Vec<usize>>> {
    let index = app.world().resource::<crate::node::DriverIndex>();
    let mut paths = index.get(graph)?.awaits.clone();
    paths.sort();
    Some(paths)
}

/// The index holds the driven nodes of each open head's committed graph, and
/// drops a graph once no open head shows it.
#[test]
fn the_index_follows_the_open_heads() {
    let mut app = super::test_app(|| {});
    let first = commit(&mut app, graph(2));
    app.world_mut()
        .trigger(head::OpenEvent(Head::Branch(name())));
    app.update();
    assert_eq!(awaits(&app, &first), Some(vec![vec![1], vec![2]]));

    let second = commit(&mut app, graph(1));
    app.update();
    assert_eq!(awaits(&app, &second), Some(vec![vec![1]]));
    assert_eq!(awaits(&app, &first), None);
}
