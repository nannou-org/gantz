//! A node that fires when a graph runs as a program.
//!
//! The `main!` nodes at the root of a graph fire together as one multi-source
//! entrypoint, see [`entrypoint`]. A `main!` within a nested graph is not a
//! source. A host such as `gantz run` sets each node's state to the program
//! arguments, then calls the entrypoint.

use gantz_core::compile::{Entrypoint, entrypoint as ep};
use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx, RegCtx, graph::Graph};
use gantz_core::steel::SteelVal;
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};
use std::any::{Any, TypeId};
use std::ops::Deref;

/// A graph's program entry.
///
/// Outputs the program arguments as a list of strings. The arguments are the
/// node's state, which is `'()` until a host sets it.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize, NodeTag)]
pub struct MainBang;

impl gantz_core::Node for MainBang {
    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        true
    }

    fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
        node::parse_expr("(begin state)")
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        let path = ctx.path();
        node::state::init_value_if_absent(ctx.vm(), path, || SteelVal::ListV(vec![].into()))
            .unwrap()
    }
}

/// The entrypoint that fires every `main!` node at the root of `graph`
/// together, or `None` if the root has no `main!` node.
pub fn entrypoint<N>(graph: &Graph<N>) -> Option<Entrypoint>
where
    N: Deref<Target: gantz_core::Node>,
{
    // `Any::type_id` dispatches through a `dyn` target to the concrete node.
    let ids = graph
        .node_indices()
        .filter(|&ix| Any::type_id(&*graph[ix]) == TypeId::of::<MainBang>())
        .map(|ix| ix.index());
    entrypoint_of(ids)
}

/// The entrypoint that fires the root `main!` nodes with the given ids
/// together, or `None` if there are none.
pub fn entrypoint_of(ids: impl IntoIterator<Item = node::Id>) -> Option<Entrypoint> {
    let sources: Vec<_> = ids
        .into_iter()
        .map(|id| ep::push_source(vec![id], 1))
        .collect();
    (!sources.is_empty()).then(|| ep::from_sources(sources))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_core::{
        Edge, Node,
        compile::{EvalKind, entry_fn_name},
    };

    fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
        None
    }

    // Two root `main!` nodes are the sources of one entrypoint. The `main!` in
    // the nested graph is not a source.
    #[test]
    fn entrypoint_covers_root_main_bangs_only() {
        let mut inner: Graph<Box<dyn Node>> = Graph::default();
        inner.add_node(Box::new(MainBang));
        let mut g: Graph<Box<dyn Node>> = Graph::default();
        let a = g.add_node(Box::new(MainBang) as Box<dyn Node>);
        g.add_node(Box::new(inner) as Box<_>);
        let b = g.add_node(Box::new(MainBang) as Box<_>);

        let ep = entrypoint(&g).expect("main! entrypoint");
        let sources: Vec<_> = ep.0.iter().map(|s| (s.kind, s.path.clone())).collect();
        assert_eq!(
            sources,
            [
                (EvalKind::Push, vec![a.index()]),
                (EvalKind::Push, vec![b.index()]),
            ]
        );

        let empty: Graph<Box<dyn Node>> = Graph::default();
        assert!(entrypoint(&empty).is_none());
    }

    // Firing the entrypoint emits each `main!` node's state downstream. The
    // state is `'()` until a host sets it. The `check` expression panics on
    // failure, so a successful fire proves the output.
    #[test]
    fn main_bang_outputs_its_state() {
        let fire = |check: &str, args: Option<Vec<String>>| {
            let mut g: Graph<Box<dyn Node>> = Graph::default();
            let main = g.add_node(Box::new(MainBang) as Box<dyn Node>);
            let check = g.add_node(Box::new(node::expr(check).unwrap()) as Box<_>);
            g.add_edge(main, check, Edge::from((0, 0)));

            let ep = entrypoint(&g).expect("main! entrypoint");
            let config = gantz_core::compile::Config::default();
            let (mut vm, _compiled) =
                gantz_core::vm::init(&no_lookup, &g, std::slice::from_ref(&ep), &config)
                    .unwrap_or_else(|e| panic!("init: {}", gantz_core::vm::error_chain(&e)));
            if let Some(args) = args {
                node::state::update(&mut vm, &[main.index()], args).expect("set args");
            }
            vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
                .unwrap_or_else(|e| panic!("firing main! errored: {e:?}"));
        };
        fire("(assert! (equal? $a '()))", None);
        let args = vec!["x".to_string(), "y".to_string()];
        fire("(assert! (equal? $a (list \"x\" \"y\")))", Some(args));
    }
}
