//! Items related to constructing a view of the control flow of a gantz graph.

use super::{EdgeKind, Meta};
use crate::node;
use petgraph::visit::EdgeRef;
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// Represents all control flow graphs for all entrypoints in a single gantz graph.
///
/// This includes all branches on the edges, and unique node configurations as
/// nodes.
#[derive(Debug)]
pub struct Flow {
    /// Control flow graph from all inlets to all outlets, or empty in the case
    /// that the graph has no inlets or outlets (i.e. is not nested).
    pub nested: FlowGraph,
    /// The control flow graph for each `push_eval` configuration for each node.
    pub push: BTreeMap<(node::Id, node::Conns), FlowGraph>,
    /// The control flow graph for each `pull_eval` configuration for each node.
    pub pull: BTreeMap<(node::Id, node::Conns), FlowGraph>,
}

/// The control flow graph.
pub type FlowGraph = petgraph::graphmap::DiGraphMap<NodeConf, BranchConns>;

/// One of the
type BranchConns = node::Conns;

/// A node within the control flow graph.
///
/// Maps directly to a node function.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
struct NodeConf {
    id: node::Id,
    conns: NodeConns,
}

/// The connectedness of a node for a particular evaluation step.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub(super) struct NodeConns {
    /// The active inputs (conditional connections may or may not be active).
    inputs: node::Conns,
    /// Includes all connected outputs (whether conditional or not).
    outputs: node::Conns,
}

impl Flow {
    fn from_meta(meta: &Meta) -> Self {
        // Create the push eval entrypoint control flow graphs.
        let push = meta
            .push
            .iter()
            .flat_map(|(&n, connss)| {
                connss
                    .iter()
                    .map(move |conns| ((n, *conns), push_eval_flow_graph(meta, n, conns)))
            })
            .collect();

        let pull = todo!();
        let nested = todo!();

        Self { nested, push, pull }
    }
}

/// Given the meta graph and a node registered as a `push_eval` entrypoint,
/// produce the control flow graph.
fn push_eval_flow_graph(meta: &Meta, n: node::Id, conns: &node::Conns) -> FlowGraph {
    use super::{push_eval_neighbors, push_eval_order};

    let mut g = FlowGraph::new();

    // Iterate over the meta nodes that are included in topo order.
    let nbs = push_eval_neighbors(&meta.graph, n, conns);
    let order: Vec<_> = push_eval_order(&meta.graph, n, &nbs).collect();
    let included: HashSet<_> = order.iter().copied().collect();

    // For each node, add a `NodeConf` for every permutation of inputs/outputs
    // required, and an edge for each input node's branches.
    let mut visited: BTreeMap<node::Id, Vec<NodeConf>> = Default::default();
    for n in order {
        for (a, b, branch) in node_input_edges(meta, n, &included, &visited) {
            g.add_edge(a, b, branch);
            visited.entry(n).or_default().push(b);
        }
    }
    g
}

/// The input edges of all configurations of this node.
///
/// All connected outputs (whether conditional or not).
fn node_input_edges(
    meta: &Meta,
    dst: node::Id,
    included: &HashSet<node::Id>,
    visited: &BTreeMap<node::Id, Vec<NodeConf>>,
) -> BTreeSet<(NodeConf, NodeConf, BranchConns)> {
    let mut input_edges = BTreeSet::new();

    // Retrieve all possible configuration permutations for this node.
    let dst_confs = node_conf_perms(meta, dst, |n| included.contains(&n));

    // For each configuragion of each source node, determine all branching edges
    // to each relevant configuration of this node.
    for e_ref in meta.graph.edges_directed(dst, petgraph::Incoming) {
        let src = e_ref.source();
        let Some(src_confs) = visited.get(&src) else {
            continue;
        };

        // For every edge between this pair, create an edge for each conf
        // permutation. Doubles will automatically be deduped.
        for (edge, kind) in e_ref.weight() {
            let src_out = edge.output.0 as usize;
            let dst_in = edge.input.0 as usize;

            for &src_conf in src_confs {
                // Skip src confs that don't touch this output.
                if !src_conf.conns.outputs.get(src_out).unwrap() {
                    continue;
                }

                for &dst_conf in &dst_confs {
                    // Skip dst confs that don't touch this input.
                    if !dst_conf.conns.inputs.get(dst_in).unwrap() {
                        continue;
                    }

                    // TODO: need to include
                    // For each of the src branches that touch this
                    let src_branches = meta.branches.get(&src).cloned().unwrap_or_else(|| {
                        todo!("node::Conns with all connected src outputs")
                    });
                    for src_branch in src_branches {
                        if src_branch.get(src_out).unwrap() {
                            input_edges.insert((src_conf, dst_conf, src_branch));
                        }
                    }
                }
            }
        }
    }

    input_edges
}

/// Retrieve all possible configuration permutations for this node.
///
/// This will include:
///
/// - All `inputs` permutations over its conditional input edges.
/// - One `outputs` for all connected output edges (conditional or not).
fn node_conf_perms(meta: &Meta, n: node::Id, included: impl Fn(node::Id) -> bool) -> Vec<NodeConf> {
    // 1. Collect all permutations of inputs as `Vec<node::Conns>`.
    let n_inputs = meta.inputs.get(&n).copied().unwrap_or(0);
    let mut input_kinds: Vec<Option<EdgeKind>> = vec![None; n_inputs];
    for e_ref in meta.graph.edges_directed(n, petgraph::Incoming) {
        // Only consider edges to nodes included in the traversal.
        if !included(e_ref.source()) {
            continue;
        }
        for (edge, kind) in e_ref.weight() {
            input_kinds[edge.input.0 as usize] = Some(kind.clone());
        }
    }
    let inputs = conns_permutations(&input_kinds);

    // 2. The only output should include all connected output edges.
    // Note: Branches are handled at runtime, so we omit those until adding edges.
    let n_outputs = meta.outputs.get(&n).copied().unwrap_or(0);
    let mut outputs = node::Conns::unconnected(n_outputs).unwrap();
    for e_ref in meta.graph.edges_directed(n, petgraph::Outgoing) {
        // Only consider edges to nodes included in the traversal.
        if !included(e_ref.target()) {
            continue;
        }
        for (edge, _kind) in e_ref.weight() {
            outputs.set(edge.output.0 as usize, true);
        }
    }

    // 3. Create a conf for each input permutation with the output.
    inputs
        .iter()
        .map(move |&inputs| {
            let conns = NodeConns { inputs, outputs };
            NodeConf { id: n, conns }
        })
        .collect()
}

/// For the given edge kinds, produce all conditional permutations.
fn conns_permutations(edges: &[Option<EdgeKind>]) -> Vec<node::Conns> {
    // Find all conditional edge positions.
    let cond_positions: Vec<usize> = edges
        .iter()
        .enumerate()
        .filter_map(|(i, edge)| match edge {
            Some(EdgeKind::Conditional) => Some(i),
            _ => None,
        })
        .collect();

    let num_conds = cond_positions.len();
    let num_perms = 1 << num_conds;
    let mut perms = Vec::with_capacity(num_perms);

    // Generate all 2^n permutations.
    for perm_ix in 0..num_perms {
        let mut perm = node::Conns::unconnected(edges.len()).unwrap();
        for (i, edge) in edges.iter().enumerate() {
            let Some(kind) = edge else {
                continue;
            };
            let value = match kind {
                EdgeKind::Static => true,
                EdgeKind::Conditional => {
                    // Find which conditional this is and check the corresponding bit.
                    let cond_ix = cond_positions.iter().position(|&pos| pos == i).unwrap();
                    (perm_ix >> cond_ix) & 1 == 1
                }
            };
            perm.set(i, value);
        }
        perms.push(perm);
    }

    perms
}

mod tests {
    use super::*;

    // Meta:
    //
    // -----
    // | 0 | // push
    // -+---
    //  |
    // -+---
    // | 1 |
    // -+-+-
    //  | |
    //  | -----  // both edges conditional
    //  |     |
    // -+--- -+---
    // | 2 | | 3 |
    // ----- -----
    //
    // Flow:
    //
    // -----
    // | 0 |
    // -+---
    //  |
    // -----
    // | 1 |
    // -----
    //
    //
    //
    //
    #[test]
    fn flow_graph() {
        // let meta = Meta {
        // graph:
        // };
    }

    #[test]
    fn test_generate_edge_permutations() {
        // Test case: [Static, Conditional, None, Conditional]
        let edges = vec![
            Some(EdgeKind::Static),
            Some(EdgeKind::Conditional),
            None,
            Some(EdgeKind::Conditional),
        ];

        let permutations = conns_permutations(&edges);

        // Should have 2^2 = 4 permutations (2 conditional edges)
        assert_eq!(permutations.len(), 4);

        // Expected permutations:
        // [true, false, false, false] - both conditionals false
        // [true, true, false, false]  - first conditional true, second false
        // [true, false, false, true]  - first conditional false, second true
        // [true, true, false, true]   - both conditionals true

        let expected = vec![
            node::Conns::try_from([true, false, false, false]).unwrap(),
            node::Conns::try_from([true, true, false, false]).unwrap(),
            node::Conns::try_from([true, false, false, true]).unwrap(),
            node::Conns::try_from([true, true, false, true]).unwrap(),
        ];

        assert_eq!(permutations, expected);
    }

    #[test]
    fn test_no_conditionals() {
        let edges = vec![Some(EdgeKind::Static), None, Some(EdgeKind::Static)];

        let permutations = conns_permutations(&edges);

        // Should have exactly 1 permutation
        assert_eq!(permutations.len(), 1);
        assert_eq!(
            permutations[0],
            node::Conns::try_from([true, false, true]).unwrap()
        );
    }

    #[test]
    fn test_all_conditionals() {
        let edges = vec![Some(EdgeKind::Conditional), Some(EdgeKind::Conditional)];

        let permutations = conns_permutations(&edges);

        // Should have 2^2 = 4 permutations
        assert_eq!(permutations.len(), 4);

        let expected = vec![
            node::Conns::try_from([false, false]).unwrap(),
            node::Conns::try_from([true, false]).unwrap(),
            node::Conns::try_from([false, true]).unwrap(),
            node::Conns::try_from([true, true]).unwrap(),
        ];

        assert_eq!(permutations, expected);
    }
}
