//! Detecting reference cycles among named graphs.
//!
//! Adding a [`NamedRef`](crate::NamedRef) to the graph it lives in would form
//! a reference cycle. The reference can be direct, or transitive through the
//! referenced graph's own named references. With `sync` enabled such a cycle
//! recommits endlessly, since a parent chases its own moving commit.
//! Creation is refused up-front. This is the live-editor counterpart of
//! `gantz_format`'s load-time `CycleInRefs` check.

use crate::named_ref::named_ref_of;
use gantz_ca::{Name, Registry};
use std::collections::HashSet;

/// Whether inserting a reference to the graph named `target` into the graph
/// named `editing` would create a reference cycle.
///
/// A cycle exists when `editing` is reachable from `target` through named
/// references at any depth. The trivial `target == editing` case counts.
/// Names that resolve to no graph, such as builtins, contribute no edges.
/// The walk reads `NamedRef` names straight off the registry's stored data
/// graphs via `named_ref_of`. No typed node set is involved.
pub fn would_cycle(registry: &Registry, target: &Name, editing: &Name) -> bool {
    let mut stack = vec![target.clone()];
    let mut visited = HashSet::new();
    while let Some(name) = stack.pop() {
        if name == *editing {
            return true;
        }
        if !visited.insert(name.clone()) {
            continue;
        }
        let Some(graph_addr) = registry.named_commit(&name).map(|c| c.graph) else {
            continue;
        };
        let Some(graph) = registry.graph(&graph_addr) else {
            continue;
        };
        for weight in graph.node_weights() {
            if let Some(named_ref) = named_ref_of(weight) {
                stack.push(named_ref.name().clone());
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NamedRef;
    use gantz_ca::{ContentAddr, DataGraph};

    fn name(s: &str) -> Name {
        s.parse().unwrap()
    }

    /// Commit a graph of `NamedRef`s under `graph_name`, one per referenced
    /// name. The walk is by name, so each ref points at a placeholder address.
    fn commit_named_refs(registry: &mut Registry, graph_name: &str, refs: &[&str]) {
        let mut graph = DataGraph::default();
        for &r in refs {
            let ref_ = gantz_core::node::Ref::new(ContentAddr::from([0; 32]));
            let named_ref = NamedRef::new(name(r), ref_);
            graph.add_node(gantz_core::data::erase_node_typed(&named_ref).unwrap());
        }
        let graph_ca = gantz_ca::graph_addr(&graph);
        registry.commit_graph_to_name(
            std::time::Duration::ZERO,
            graph_ca,
            || graph,
            &name(graph_name),
        );
    }

    #[test]
    fn detects_cycles_by_name() {
        let mut registry = Registry::default();
        // `a` and `b` reference each other.
        commit_named_refs(&mut registry, "b", &[]);
        commit_named_refs(&mut registry, "a", &["b"]);
        commit_named_refs(&mut registry, "b", &["a"]);
        // An unrelated standalone graph.
        commit_named_refs(&mut registry, "c", &[]);

        // Self-reference.
        assert!(would_cycle(&registry, &name("a"), &name("a")));
        // `b` reaches `a`, so referencing `b` from `a` closes the loop.
        assert!(would_cycle(&registry, &name("b"), &name("a")));
        // `c` references nothing, so it is safe.
        assert!(!would_cycle(&registry, &name("c"), &name("a")));
        // An unknown or builtin name resolves to no graph, so it is safe.
        assert!(!would_cycle(&registry, &name("not-a-name"), &name("a")));
    }
}
