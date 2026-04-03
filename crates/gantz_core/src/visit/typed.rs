//! Typed visitor support for downcasting `&dyn Node` to concrete types.
//!
//! [`TypedVisitor<N>`] mirrors [`Visitor`] but receives `&N` instead of
//! `&dyn Node`. The [`Typed`] adapter wraps a `TypedVisitor<N>` into a
//! `Visitor` by downcasting each visited node via [`Any`].

use super::{Ctx, Visitor};
use crate::node::Node;
use std::any::Any;
use std::marker::PhantomData;

/// A visitor that receives concrete `&N` instead of `&dyn Node`.
///
/// Implement this trait to handle only nodes of a specific concrete type
/// during traversal. Nodes that don't match `N` are silently skipped.
pub trait TypedVisitor<N> {
    /// Called prior to traversing nested nodes.
    fn visit_pre(&mut self, _ctx: Ctx<'_, '_>, _node: &N) {}
    /// Called following traversal of nested nodes.
    fn visit_post(&mut self, _ctx: Ctx<'_, '_>, _node: &N) {}
}

/// Adapts a [`TypedVisitor<N>`] into a [`Visitor`] via [`Any`] downcasting.
///
/// Nodes whose concrete type is not `N` are silently skipped.
pub struct Typed<V, N> {
    /// The inner typed visitor.
    pub visitor: V,
    _marker: PhantomData<fn() -> N>,
}

impl<V: ?Sized + TypedVisitor<N>, N> TypedVisitor<N> for &mut V {
    fn visit_pre(&mut self, ctx: Ctx<'_, '_>, node: &N) {
        (**self).visit_pre(ctx, node);
    }
    fn visit_post(&mut self, ctx: Ctx<'_, '_>, node: &N) {
        (**self).visit_post(ctx, node);
    }
}

impl<V, N> Typed<V, N> {
    /// Wrap a [`TypedVisitor<N>`] as a [`Visitor`].
    pub fn new(visitor: V) -> Self {
        Self {
            visitor,
            _marker: PhantomData,
        }
    }

    /// Unwrap, returning the inner visitor.
    pub fn into_inner(self) -> V {
        self.visitor
    }
}

impl<V: TypedVisitor<N>, N: 'static> Visitor for Typed<V, N> {
    fn visit_pre(&mut self, ctx: Ctx<'_, '_>, node: &dyn Node) {
        let any: &dyn Any = node;
        if let Some(n) = any.downcast_ref::<N>() {
            self.visitor.visit_pre(ctx, n);
        }
    }
    fn visit_post(&mut self, ctx: Ctx<'_, '_>, node: &dyn Node) {
        let any: &dyn Any = node;
        if let Some(n) = any.downcast_ref::<N>() {
            self.visitor.visit_post(ctx, n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{self, ExprCtx, ExprResult, MetaCtx, Node};
    use crate::visit;

    /// A concrete node type for testing typed visitors.
    #[derive(Debug)]
    struct TestNode {
        label: String,
    }

    impl Node for TestNode {
        fn n_inputs(&self, _ctx: MetaCtx) -> usize {
            0
        }
        fn n_outputs(&self, _ctx: MetaCtx) -> usize {
            1
        }
        fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
            node::parse_expr("42")
        }
    }

    fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
        None
    }

    struct LabelCollector {
        labels: Vec<String>,
    }

    impl TypedVisitor<TestNode> for LabelCollector {
        fn visit_pre(&mut self, _ctx: Ctx<'_, '_>, node: &TestNode) {
            self.labels.push(node.label.clone());
        }
    }

    #[test]
    fn typed_visitor_downcasts_matching_nodes() {
        let mut collector = LabelCollector { labels: vec![] };
        let ctx = visit::Ctx::new(&no_lookup, &[0], &[]);
        let test_node = TestNode {
            label: "hello".into(),
        };

        node::visit_typed::<_, TestNode>(ctx, &test_node, &mut collector);
        assert_eq!(collector.labels, vec!["hello"]);
    }

    #[test]
    fn graph_visit_typed_collects_matching_nodes() {
        use crate::Edge;
        use crate::graph;
        use crate::node::graph::Graph;

        // Use a concrete node type directly so that downcast succeeds.
        let mut g: Graph<TestNode> = Graph::default();
        let a = g.add_node(TestNode { label: "A".into() });
        let b = g.add_node(TestNode { label: "B".into() });
        let c = g.add_node(TestNode { label: "C".into() });
        g.add_edge(a, b, Edge::new(0.into(), 0.into()));
        g.add_edge(b, c, Edge::new(0.into(), 0.into()));

        let mut collector = LabelCollector { labels: vec![] };
        graph::visit_typed(&no_lookup, &g, &[], &mut collector);
        assert_eq!(collector.labels, vec!["A", "B", "C"]);
    }
}
