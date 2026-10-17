//! Tests that the node wrappers forward the wrapped node's role and nested
//! nodes.

use gantz_core::node::graph::Graph;
use gantz_core::node::{
    self, Delay, MetaCtx, Node, NodeState, Pull, Push, Role, State, Visitor, visit,
};
use steel::steel_vm::engine::Engine;
use steel_derive::Steel;

/// A state type with no fns, for wrapping in [`State`].
#[derive(Clone, Debug, Default, Steel)]
struct Empty;

impl NodeState for Empty {
    const NAME: &str = "Empty";
    fn register_fns(_vm: &mut Engine) {}
}

/// Counts the nodes that a visit reaches.
#[derive(Default)]
struct Count(usize);

impl Visitor for Count {
    fn visit_pre(&mut self, _ctx: visit::Ctx<'_, '_>, _node: &dyn Node) {
        self.0 += 1;
    }
}

fn no_node(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
    None
}

/// A graph node with two nested nodes.
fn nested() -> Graph<Box<dyn Node>> {
    let mut g: Graph<Box<dyn Node>> = Graph::default();
    g.add_node(Box::new(node::expr("1").unwrap()));
    g.add_node(Box::new(node::expr("2").unwrap()));
    g
}

/// The number of nodes that a visit from `node` reaches, `node` included.
fn visited(node: &dyn Node) -> usize {
    let mut count = Count::default();
    node::visit(visit::Ctx::new(&no_node, &[0], &[]), node, &mut count);
    count.0
}

fn role(node: &dyn Node) -> Role {
    node.role(MetaCtx::new(&no_node))
}

#[test]
fn push_forwards_role_and_visit() {
    assert_eq!(role(&Push::all(Delay)), Role::Delay);
    assert_eq!(visited(&Push::all(nested())), 3);
}

#[test]
fn pull_forwards_role_and_visit() {
    assert_eq!(role(&Pull::all(Delay)), Role::Delay);
    assert_eq!(visited(&Pull::all(nested())), 3);
}

#[test]
fn state_forwards_role_and_visit() {
    assert_eq!(role(&State::<_, Empty>::new(Delay)), Role::Delay);
    assert_eq!(visited(&State::<_, Empty>::new(nested())), 3);
}
