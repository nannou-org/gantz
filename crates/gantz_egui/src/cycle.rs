//! Detecting reference cycles among named graphs.
//!
//! Adding a [`NamedRef`] to the graph it lives in -
//! directly, or transitively through the referenced graph's own named
//! references - would form a reference cycle. With `sync` enabled such a cycle
//! recommits endlessly (a parent chases its own moving commit), so creation is
//! refused up-front. This is the live-editor counterpart of `gantz_format`'s
//! load-time `CycleInRefs` check.

use crate::node::NamedRef;
use gantz_ca::{Name, NodeData, Registry};
use gantz_nodetag::NodeTag;
use std::collections::HashSet;

/// The [`NamedRef`] stored in `weight`, if it is one.
///
/// Tag-gated: only a weight whose tag is exactly `NamedRef`'s matches (never
/// `Fn`/`FnNamedRef`, which reference a graph without standing in for it). A
/// tag-matched weight that fails to decode is logged and treated as no
/// reference.
pub(crate) fn named_ref_of(weight: &NodeData) -> Option<NamedRef> {
    if weight.tag != <NamedRef as NodeTag>::TAG {
        return None;
    }
    match gantz_core::data::reify_node_concrete::<NamedRef>(weight) {
        Ok(named_ref) => Some(named_ref),
        Err(e) => {
            log::error!("failed to decode a stored `NamedRef`: {e}");
            None
        }
    }
}

/// Whether inserting a reference to the graph named `target` into the graph
/// named `editing` would create a reference cycle.
///
/// A cycle exists when `editing` is reachable from `target` through named
/// references at any depth - including the trivial `target == editing`. Names
/// that resolve to no graph (e.g. builtins) simply contribute no edges. The
/// walk reads `NamedRef` names straight off the registry's stored data
/// graphs (see `named_ref_of`) - no typed node set involved.
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
    use crate::test_node::{TestGraph, named_ref};

    fn name(s: &str) -> Name {
        s.parse().unwrap()
    }

    /// Commit a graph of `NamedRef`s (one per referenced name) under `name`.
    fn commit_named_refs(registry: &mut Registry, graph_name: &str, refs: &[&str]) {
        let mut graph = TestGraph::default();
        for &r in refs {
            // The referenced content address is irrelevant to the name-based
            // walk; point each ref at the target name's head graph if known,
            // else a placeholder derived from an empty graph.
            let ga: gantz_ca::GraphAddr = registry
                .named_commit(&name(r))
                .map(|c| c.graph)
                .unwrap_or_else(|| {
                    gantz_core::data::erase_with_addr(&TestGraph::default())
                        .unwrap()
                        .1
                });
            graph.add_node(named_ref(r, ga));
        }
        let (data_graph, graph_ca) = gantz_core::data::erase_with_addr(&graph).unwrap();
        registry.commit_graph_to_name(
            std::time::Duration::ZERO,
            graph_ca,
            || data_graph,
            &name(graph_name),
        );
    }

    #[test]
    fn detects_cycles_by_name() {
        let mut registry = Registry::default();
        // `a` references `b`; `b` references `a`.
        commit_named_refs(&mut registry, "b", &[]);
        commit_named_refs(&mut registry, "a", &["b"]);
        commit_named_refs(&mut registry, "b", &["a"]);
        // An unrelated standalone graph.
        commit_named_refs(&mut registry, "c", &[]);

        // Self-reference.
        assert!(would_cycle(&registry, &name("a"), &name("a")));
        // `b` reaches `a`, so referencing `b` from `a` closes the loop.
        assert!(would_cycle(&registry, &name("b"), &name("a")));
        // `c` references nothing - safe.
        assert!(!would_cycle(&registry, &name("c"), &name("a")));
        // An unknown / builtin name resolves to no graph - safe.
        assert!(!would_cycle(&registry, &name("not-a-name"), &name("a")));
    }
}
