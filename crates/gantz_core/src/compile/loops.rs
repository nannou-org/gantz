//! Analysis of feedback loops (directed cycles) within one graph level.
//!
//! gantz lowers a cyclic graph as an *iterate-until-branch* loop: a single
//! evaluation re-runs the cyclic region until a branch node selects an arm that
//! exits. This module finds the loops in a [`MetaGraph`], identifying for each
//! the single entry ("header"), the back-edges that close it, the loop-carried
//! values, and which branch arms continue (re-enter) vs exit the loop.
//!
//! Detection is deterministic - every result is keyed/sorted by [`node::Id`] -
//! so the generated code is reproducible (gantz is content-addressed).
//!
//! Loops must be *reducible* (single-entry) and contain at least one branch with
//! an arm that exits; violations are reported as [`LoopError`]. Termination
//! itself is dynamic and is *not* statically verified - a loop with an exit arm
//! may still run forever depending on runtime branch decisions (that is the
//! user's responsibility, as with any hand-written `while`).
//!
//! Nested loops are found by residual-SCC recursion: after recording a loop, its
//! body (minus its back-edges) is re-analyzed to expose inner loops, stored flat
//! in the table keyed by their own header.

use super::{MetaGraph, error::LoopError};
use crate::{Edge, node};
use petgraph::{algo::tarjan_scc, visit::EdgeRef};
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// A loop-carried parameter: a header input whose value is updated each
/// iteration via a back-edge (it becomes a tail-recursive function parameter).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LoopParam {
    /// The header input index carried across iterations.
    pub header_input: usize,
    /// The external (pre-loop) source feeding this input on the first iteration,
    /// or `None` when the input is fed only by back-edges.
    pub initial: Option<(node::Id, node::Output)>,
}

/// A single reducible (single-entry) feedback loop.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LoopInfo {
    /// The loop header: the single entry node and target of every back-edge.
    pub header: node::Id,
    /// Every node in the loop's strongly-connected component.
    pub body: BTreeSet<node::Id>,
    /// Back-edges (intra-SCC edges into the header), each with its full [`Edge`].
    pub back_edges: Vec<(node::Id, Edge)>,
    /// Loop-carried params, one per header input fed by a back-edge (ascending).
    pub carried: Vec<LoopParam>,
    /// For each branch node within the loop, the arm indices that re-enter it.
    pub continue_arms: BTreeMap<node::Id, BTreeSet<usize>>,
}

/// All loops at one graph level, keyed by header id.
pub(crate) type LoopTable = BTreeMap<node::Id, LoopInfo>;

/// Find all feedback loops in `mg`, validating that each is reducible and can
/// terminate. `branches` maps each branching node to its per-arm output masks.
pub(crate) fn analyze(
    mg: &MetaGraph,
    branches: &BTreeMap<node::Id, Vec<node::Conns>>,
) -> Result<LoopTable, LoopError> {
    let mut table = LoopTable::new();
    for scc in tarjan_scc(mg) {
        let cyclic = scc.len() > 1 || (scc.len() == 1 && mg.contains_edge(scc[0], scc[0]));
        if !cyclic {
            continue;
        }
        let body: BTreeSet<node::Id> = scc.iter().copied().collect();
        let header = find_header(mg, &body)?;
        let back_edges = back_edges_into(mg, &body, header);

        let continue_arms = classify_arms(&body, &back_edges, branches)?;
        let carried = carried_params(mg, &body, header, &back_edges);
        // Recurse into the residual (this loop's body with its back-edges removed)
        // to discover nested inner loops. They are stored flat in the table, keyed
        // by their own header; nesting is recovered from `body` containment.
        let removed: HashSet<(node::Id, node::Id)> =
            back_edges.iter().map(|(src, _)| (*src, header)).collect();
        let residual = sub_graph(mg, &body, &removed);
        table.insert(
            header,
            LoopInfo {
                header,
                body,
                back_edges,
                carried,
                continue_arms,
            },
        );
        table.extend(analyze(&residual, branches)?);
    }
    Ok(table)
}

/// The single node of `scc` entered from outside it (the loop header). Errors if
/// there is not exactly one such node (multi-entry / no-entry = irreducible).
fn find_header(mg: &MetaGraph, scc: &BTreeSet<node::Id>) -> Result<node::Id, LoopError> {
    let mut entries: BTreeSet<node::Id> = BTreeSet::new();
    for &n in scc {
        let entered_externally = mg
            .edges_directed(n, petgraph::Incoming)
            .any(|e| !scc.contains(&e.source()));
        if entered_externally {
            entries.insert(n);
        }
    }
    if entries.len() == 1 {
        Ok(entries.into_iter().next().unwrap())
    } else {
        Err(LoopError::IrreducibleLoop {
            nodes: scc.iter().copied().collect(),
            entries: entries.into_iter().collect(),
        })
    }
}

/// The loop's back-edges: intra-SCC edges into `header`, each with its [`Edge`].
fn back_edges_into(
    mg: &MetaGraph,
    scc: &BTreeSet<node::Id>,
    header: node::Id,
) -> Vec<(node::Id, Edge)> {
    let mut out = Vec::new();
    for e in mg.edges_directed(header, petgraph::Incoming) {
        let src = e.source();
        if scc.contains(&src) {
            out.extend(e.weight().iter().map(|(edge, _kind)| (src, *edge)));
        }
    }
    out.sort();
    out
}

/// Loop-carried params: each header input fed by a back-edge, paired with its
/// external (pre-loop) initial source if any.
fn carried_params(
    mg: &MetaGraph,
    scc: &BTreeSet<node::Id>,
    header: node::Id,
    back_edges: &[(node::Id, Edge)],
) -> Vec<LoopParam> {
    let carried_inputs: BTreeSet<usize> =
        back_edges.iter().map(|(_, e)| e.input.0 as usize).collect();
    carried_inputs
        .into_iter()
        .map(|input| {
            // The initial value is the in-edge to this input from outside the SCC.
            let mut initial = None;
            for e in mg.edges_directed(header, petgraph::Incoming) {
                let src = e.source();
                if scc.contains(&src) {
                    continue; // a back-edge, not the initial seed
                }
                for (edge, _kind) in e.weight() {
                    if edge.input.0 as usize == input {
                        initial = Some((src, edge.output));
                    }
                }
            }
            LoopParam {
                header_input: input,
                initial,
            }
        })
        .collect()
}

/// For each branch that drives a back-edge, the arm indices that re-enter the
/// loop (i.e. activate a back-edge's output). Errors if no branch gates a
/// back-edge, or if every deciding branch always re-enters (no exit arm), so the
/// loop can never terminate.
///
/// v1 requires the back-edge to originate at a branch node, so the branch
/// directly decides continue-vs-exit (the natural counter/accumulator shape). A
/// loop whose back-edge is unconditional from its source is reported as
/// non-terminating.
fn classify_arms(
    scc: &BTreeSet<node::Id>,
    back_edges: &[(node::Id, Edge)],
    branches: &BTreeMap<node::Id, Vec<node::Conns>>,
) -> Result<BTreeMap<node::Id, BTreeSet<usize>>, LoopError> {
    // The set of output indices that drive a back-edge, per source node.
    let mut back_outputs: BTreeMap<node::Id, BTreeSet<usize>> = BTreeMap::new();
    for (src, edge) in back_edges {
        back_outputs
            .entry(*src)
            .or_default()
            .insert(edge.output.0 as usize);
    }

    // The branches in this loop that directly drive a back-edge.
    let deciding: Vec<node::Id> = scc
        .iter()
        .copied()
        .filter(|n| branches.contains_key(n) && back_outputs.contains_key(n))
        .collect();
    if deciding.is_empty() {
        return Err(LoopError::InfiniteFeedbackLoop {
            nodes: scc.iter().copied().collect(),
        });
    }
    // A loop may contain any number of inner forward branches (which reconverge
    // within the body); v1 supports only a single *deciding* branch - the one exit
    // decision. Multiple deciding branches (multi-exit) need a different lowering.
    if deciding.len() > 1 {
        return Err(LoopError::MultiExitLoopUnsupported {
            nodes: scc.iter().copied().collect(),
        });
    }

    let mut continue_arms = BTreeMap::new();
    let mut terminable = false;
    for b in deciding {
        let outs = &back_outputs[&b];
        let mut continues = BTreeSet::new();
        for (ix, conns) in branches[&b].iter().enumerate() {
            // An arm re-enters the loop iff it activates a back-edge's output.
            let takes_back_edge = outs.iter().any(|&o| conns.get(o).unwrap_or(false));
            if takes_back_edge {
                continues.insert(ix);
            } else {
                terminable = true;
            }
        }
        continue_arms.insert(b, continues);
    }
    if !terminable {
        return Err(LoopError::InfiniteFeedbackLoop {
            nodes: scc.iter().copied().collect(),
        });
    }
    Ok(continue_arms)
}

/// The subgraph of `mg` induced by `nodes`, minus any `removed` edges. Isolated
/// nodes are preserved so the node set is exactly `nodes`.
fn sub_graph(
    mg: &MetaGraph,
    nodes: &BTreeSet<node::Id>,
    removed: &HashSet<(node::Id, node::Id)>,
) -> MetaGraph {
    let mut g: MetaGraph = mg
        .all_edges()
        .filter(|(a, b, _)| nodes.contains(a) && nodes.contains(b) && !removed.contains(&(*a, *b)))
        .map(|(a, b, w)| (a, b, w.clone()))
        .collect();
    for &n in nodes {
        if !g.contains_node(n) {
            g.add_node(n);
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::EdgeKind;

    /// A `Static` edge weight from output `out` to input `inp`.
    fn edge(out: u16, inp: u16) -> Vec<(Edge, EdgeKind)> {
        vec![(Edge::new(out.into(), inp.into()), EdgeKind::Static)]
    }

    /// A `Conns` from a bit string, e.g. `conns("10")`.
    fn conns(bits: &str) -> node::Conns {
        bits.parse().unwrap()
    }

    /// A counter-style loop:
    ///   0(seed) -> 1(add, header) -> 2(branch);
    ///   2.o0 -> 1 (back-edge, arm0);  2.o1 -> 3(out) (exit, arm1).
    #[test]
    fn counter_loop() {
        let mut g = MetaGraph::new();
        g.add_edge(0, 1, edge(0, 0)); // seed -> add
        g.add_edge(1, 2, edge(0, 0)); // add -> branch
        g.add_edge(2, 1, edge(0, 0)); // branch.o0 -> add (back-edge)
        g.add_edge(2, 3, edge(1, 0)); // branch.o1 -> out (exit)
        let branches = BTreeMap::from([(2, vec![conns("10"), conns("01")])]);

        let table = analyze(&g, &branches).unwrap();
        assert_eq!(table.len(), 1);
        let info = &table[&1];
        assert_eq!(info.header, 1);
        assert_eq!(info.body, BTreeSet::from([1, 2]));
        assert_eq!(info.back_edges, vec![(2, Edge::new(0.into(), 0.into()))]);
        assert_eq!(
            info.carried,
            vec![LoopParam {
                header_input: 0,
                initial: Some((0, node::Output(0))),
            }]
        );
        assert_eq!(
            info.continue_arms,
            BTreeMap::from([(2, BTreeSet::from([0]))])
        );
    }

    /// A self-loop on a branch node: 0(seed) -> 1(branch); 1.o0 -> 1 (back),
    /// 1.o1 -> 2 (exit).
    #[test]
    fn self_loop() {
        let mut g = MetaGraph::new();
        g.add_edge(0, 1, edge(0, 0));
        g.add_edge(1, 1, edge(0, 0)); // self back-edge
        g.add_edge(1, 2, edge(1, 0)); // exit
        let branches = BTreeMap::from([(1, vec![conns("10"), conns("01")])]);

        let table = analyze(&g, &branches).unwrap();
        let info = &table[&1];
        assert_eq!(info.header, 1);
        assert_eq!(info.body, BTreeSet::from([1]));
        assert_eq!(info.back_edges, vec![(1, Edge::new(0.into(), 0.into()))]);
        assert_eq!(
            info.continue_arms,
            BTreeMap::from([(1, BTreeSet::from([0]))])
        );
    }

    /// A cycle with no branch node never terminates.
    #[test]
    fn no_branch_is_infinite() {
        let mut g = MetaGraph::new();
        g.add_edge(0, 1, edge(0, 0));
        g.add_edge(1, 2, edge(0, 0));
        g.add_edge(2, 1, edge(0, 0)); // back-edge, no branch anywhere
        let branches = BTreeMap::new();

        let err = analyze(&g, &branches).unwrap_err();
        assert!(matches!(err, LoopError::InfiniteFeedbackLoop { .. }));
    }

    /// A branch whose every arm re-enters the loop never terminates.
    #[test]
    fn all_arms_continue_is_infinite() {
        let mut g = MetaGraph::new();
        g.add_edge(0, 1, edge(0, 0)); // seed -> header (input 0)
        g.add_edge(1, 2, edge(0, 0)); // header -> branch
        // Both back-edges share the (2 -> 1) node pair, so - as `Meta::add_node`
        // does - they live in a single weight Vec (a second `add_edge` would
        // overwrite, not append).
        g.add_edge(
            2,
            1,
            vec![
                (Edge::new(0.into(), 0.into()), EdgeKind::Static), // arm0 -> input 0
                (Edge::new(1.into(), 1.into()), EdgeKind::Static), // arm1 -> input 1
            ],
        );
        let branches = BTreeMap::from([(2, vec![conns("10"), conns("01")])]);

        let err = analyze(&g, &branches).unwrap_err();
        assert!(matches!(err, LoopError::InfiniteFeedbackLoop { .. }));
    }

    /// An SCC entered at two distinct nodes is irreducible.
    #[test]
    fn multi_entry_is_irreducible() {
        let mut g = MetaGraph::new();
        g.add_edge(0, 1, edge(0, 0)); // external entry to 1
        g.add_edge(3, 2, edge(0, 0)); // external entry to 2
        g.add_edge(1, 2, edge(0, 0));
        g.add_edge(2, 1, edge(0, 0));
        let branches = BTreeMap::from([(1, vec![conns("1")])]);

        let err = analyze(&g, &branches).unwrap_err();
        match err {
            LoopError::IrreducibleLoop { entries, .. } => assert_eq!(entries, vec![1, 2]),
            other => panic!("expected IrreducibleLoop, got {other:?}"),
        }
    }

    /// Two independent loops are found as two separate entries.
    #[test]
    fn two_independent_loops() {
        let mut g = MetaGraph::new();
        // Loop A: 0 -> 1 -> 2(branch) -> 1 / -> 9
        g.add_edge(0, 1, edge(0, 0));
        g.add_edge(1, 2, edge(0, 0));
        g.add_edge(2, 1, edge(0, 0));
        g.add_edge(2, 9, edge(1, 0));
        // Loop B: 9 -> 4 -> 5(branch) -> 4 / -> 6
        g.add_edge(9, 4, edge(0, 0));
        g.add_edge(4, 5, edge(0, 0));
        g.add_edge(5, 4, edge(0, 0));
        g.add_edge(5, 6, edge(1, 0));
        let branches = BTreeMap::from([
            (2, vec![conns("10"), conns("01")]),
            (5, vec![conns("10"), conns("01")]),
        ]);

        let table = analyze(&g, &branches).unwrap();
        assert_eq!(table.keys().copied().collect::<Vec<_>>(), vec![1, 4]);
    }

    /// Nested loops are found as two entries, the inner body contained in the
    /// outer, each keyed by its own header with its own deciding branch.
    #[test]
    fn nested_loops() {
        let mut g = MetaGraph::new();
        // Outer header 1, inner header 2, inner branch 3, outer branch 4.
        g.add_edge(0, 1, edge(0, 0));
        g.add_edge(1, 2, edge(0, 0));
        g.add_edge(2, 3, edge(0, 0));
        g.add_edge(3, 2, edge(0, 0)); // inner back-edge
        g.add_edge(3, 4, edge(1, 0)); // inner exit -> outer branch
        g.add_edge(4, 1, edge(0, 0)); // outer back-edge
        g.add_edge(4, 5, edge(1, 0)); // outer exit
        let branches = BTreeMap::from([
            (3, vec![conns("10"), conns("01")]),
            (4, vec![conns("10"), conns("01")]),
        ]);

        let table = analyze(&g, &branches).unwrap();
        assert_eq!(table.keys().copied().collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(table[&1].body, BTreeSet::from([1, 2, 3, 4]));
        assert_eq!(table[&2].body, BTreeSet::from([2, 3]));
        assert!(table[&2].body.is_subset(&table[&1].body));
        // The outer loop is decided by branch 4, the inner by branch 3.
        assert_eq!(
            table[&1].continue_arms.keys().copied().collect::<Vec<_>>(),
            vec![4]
        );
        assert_eq!(
            table[&2].continue_arms.keys().copied().collect::<Vec<_>>(),
            vec![3]
        );
    }
}
