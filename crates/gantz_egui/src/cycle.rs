//! Detecting reference cycles among named graphs.
//!
//! Adding a [`NamedRef`](crate::node::NamedRef) to the graph it lives in -
//! directly, or transitively through the referenced graph's own named
//! references - would form a reference cycle. With `sync` enabled such a cycle
//! recommits endlessly (a parent chases its own moving commit), so creation is
//! refused up-front. This is the live-editor counterpart of `gantz_format`'s
//! load-time `CycleInRefs` check.

use crate::sync::AsNamedRef;
use gantz_ca::Registry;
use gantz_core::node::graph::Graph;
use std::collections::HashSet;

/// Whether inserting a reference to the graph named `target` into the graph
/// named `editing` would create a reference cycle.
///
/// A cycle exists when `editing` is reachable from `target` through named
/// references at any depth - including the trivial `target == editing`. Names
/// that resolve to no graph (e.g. builtins) simply contribute no edges.
pub fn would_cycle<N>(registry: &Registry<Graph<N>>, target: &str, editing: &str) -> bool
where
    N: AsNamedRef,
{
    let mut stack = vec![target];
    let mut visited = HashSet::new();
    while let Some(name) = stack.pop() {
        if name == editing {
            return true;
        }
        if !visited.insert(name) {
            continue;
        }
        let Some(&commit) = registry.names().get(name) else {
            continue;
        };
        let Some(graph) = registry.commit_graph_ref(&commit) else {
            continue;
        };
        for weight in graph.node_weights() {
            if let Some(named_ref) = weight.as_named_ref() {
                stack.push(named_ref.name());
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::NamedRef;

    /// Commit a graph of `NamedRef`s (one per referenced name) under `name`.
    fn commit_named_refs(registry: &mut Registry<Graph<NamedRef>>, name: &str, refs: &[&str]) {
        let mut graph = Graph::<NamedRef>::default();
        for &r in refs {
            // The referenced content address is irrelevant to the name-based
            // walk; point each ref at the target name's current commit if known,
            // else a placeholder derived from an empty graph.
            let ca: gantz_ca::ContentAddr = registry
                .names()
                .get(r)
                .copied()
                .map(Into::into)
                .unwrap_or_else(|| gantz_ca::graph_addr(&Graph::<NamedRef>::default()).into());
            graph.add_node(NamedRef::new(r.to_string(), gantz_core::node::Ref::new(ca)));
        }
        let graph_ca = gantz_ca::graph_addr(&graph);
        registry.commit_graph_to_name(std::time::Duration::ZERO, graph_ca, || graph, name);
    }

    #[test]
    fn detects_cycles_by_name() {
        let mut registry = Registry::<Graph<NamedRef>>::default();
        // `a` references `b`; `b` references `a`.
        commit_named_refs(&mut registry, "b", &[]);
        commit_named_refs(&mut registry, "a", &["b"]);
        commit_named_refs(&mut registry, "b", &["a"]);
        // An unrelated standalone graph.
        commit_named_refs(&mut registry, "c", &[]);

        // Self-reference.
        assert!(would_cycle(&registry, "a", "a"));
        // `b` reaches `a`, so referencing `b` from `a` closes the loop.
        assert!(would_cycle(&registry, "b", "a"));
        // `c` references nothing - safe.
        assert!(!would_cycle(&registry, "c", "a"));
        // An unknown / builtin name resolves to no graph - safe.
        assert!(!would_cycle(&registry, "not-a-name", "a"));
    }
}
