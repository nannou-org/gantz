//! Lowering one graph level (via its [`Meta`]) to an IR [`Body`].
//!
//! Nodes are emitted as steps in a deterministic dependency order (ready
//! nodes by ascending id, branches last so as much independent work as
//! possible precedes each dispatch). At a branch:
//!
//! - each arm's region (nodes reachable only while that arm is live) lowers
//!   into the arm's body, recursively;
//! - nodes every live arm *unconditionally* reaches reconverge: they lower
//!   once into a join point, and each consumed input whose sources are
//!   arm-varying becomes a join parameter (arms pass their value, or `'()`
//!   when they don't produce one);
//! - values consumed *outside* the branch construct - by deferred nodes,
//!   enclosing scopes, or this level's outlets - flow out as branch exports
//!   (the join returns them, the dispatch statement binds them). This
//!   subsumes the old cross-component root ordering and outlet bridges.
//!
//! Dead arms (reaching nothing) yield the missing value for every export -
//! `'()`, or the unfired sentinel for outlet-feeding exports so a level
//! result can distinguish "didn't fire" from "fired with `'()`" - and bypass
//! the join, so reconvergent work runs only when a live arm actually jumps.
//!
//! A level is lowered the same way whether entered from an entrypoint
//! ([`LevelSources::Eval`]) or as a graph fn ([`LevelSources::Inlets`]);
//! inlets resolve as pre-bound parameter values and outlet values are
//! returned to the caller via [`LevelOut`].

use crate::{
    compile::{
        Meta, MetaGraph,
        error::LowerError,
        flow::{NodeConf, NodeConns, reachable_subgraph},
        ir::{Arg, Arm, Atom, Body, Join, NodeCall, Step, Subject, Tail, Var},
    },
    node,
};
use petgraph::visit::EdgeRef;
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// The fixed context for lowering one level.
pub(crate) struct Cx<'a> {
    pub meta: &'a Meta,
    /// Ids of this level's nested-graph nodes: their calls target graph fns.
    pub nested: BTreeSet<node::Id>,
    /// Additional per-entrypoint branch masks (bridged children whose inner
    /// push reaches their outlets through branching), atop `meta.branches`.
    pub extra_branches: BTreeMap<node::Id, Vec<node::Conns>>,
    /// Nodes already evaluated by enclosing glue: their `(branch-ix value)`
    /// pair ([`Var::Result`]) or outputs are bound before the body runs.
    pub prebound: BTreeSet<node::Id>,
}

/// What drives a level's evaluation.
pub(crate) enum LevelSources {
    /// A graph-fn variant: these inlets are active. All-active additionally
    /// pulls from the outlets; a subset pushes from the active inlets plus
    /// the level's static sources only (matching the flow pipeline).
    Inlets(BTreeSet<node::Id>),
    /// Entrypoint sources resolved at this level (including bridged
    /// children: pre-evaluated nodes pushing over their produced outputs).
    Eval {
        push: Vec<(node::Id, node::Conns)>,
        pull: Vec<(node::Id, node::Conns)>,
    },
}

/// One outlet's resolved value after lowering a level.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OutletVal {
    /// The atom holding the value in the body's final scope, or `None` when
    /// this variant's evaluation can never produce it.
    pub atom: Option<Atom>,
    /// Whether the value may be the unfired sentinel at runtime (it flowed
    /// through a branch export).
    pub conditional: bool,
}

/// A lowered level: its body and the values of its outlets (in id order).
pub(crate) struct LevelOut {
    pub body: Body,
    pub outlets: Vec<OutletVal>,
}

/// The lexical environment during lowering: which atom currently holds each
/// value, and per-input overrides where arm-varying sources were merged into
/// a single join parameter or export.
#[derive(Clone, Default)]
struct Env {
    /// (node, output) -> the in-scope atom holding that value.
    vals: BTreeMap<(node::Id, usize), Atom>,
    /// (node, input) -> the atom standing in for all of that input's sources.
    inputs: BTreeMap<(node::Id, usize), Atom>,
}

impl<'a> Cx<'a> {
    /// The branch arm masks of `n`, when branching (entrypoint-specific
    /// bridged branches take precedence over the graph-wide set).
    fn branches(&self, n: node::Id) -> Option<&Vec<node::Conns>> {
        self.extra_branches.get(&n).or(self.meta.branches.get(&n))
    }
}

/// Lower one level's evaluation.
pub(crate) fn level_body(cx: &Cx, sources: &LevelSources) -> Result<LevelOut, LowerError> {
    let meta = cx.meta;
    let conn1 = || node::Conns::connected(1).unwrap();

    // Reachability sources.
    let (push, pull) = match sources {
        LevelSources::Inlets(active) => {
            let mut push: Vec<(node::Id, node::Conns)> =
                active.iter().map(|&n| (n, conn1())).collect();
            if active.len() == meta.inlets.len() {
                // All inlets active: inlets push, outlets pull.
                let pull = meta.outlets.iter().map(|&n| (n, conn1())).collect();
                (push, pull)
            } else {
                // Subset: static sources stay live; no outlet pull, so an
                // unfired inlet's exclusive subtree is excluded.
                push.extend(static_sources(meta)?);
                (push, vec![])
            }
        }
        LevelSources::Eval { push, pull } => (push.clone(), pull.clone()),
    };

    // Evaluation never propagates *through* a delay (its value crosses
    // between evaluations), so ordering and reachability run on a graph with
    // delay out-edges stripped - which is also what legalizes cycles passing
    // through one. A delay whose stored value is consumed by a reached node
    // joins the reach so the read binding exists.
    let mut reach: HashSet<node::Id> = if meta.delays.is_empty() {
        super::eval_order(&meta.graph, push, pull).collect()
    } else {
        let stripped = strip_delay_out_edges(&meta.graph, &meta.delays);
        super::eval_order(&stripped, push, pull).collect()
    };
    for &d in &meta.delays {
        let consumed = meta
            .graph
            .edges_directed(d, petgraph::Outgoing)
            .any(|e_ref| reach.contains(&e_ref.target()));
        if consumed {
            reach.insert(d);
        }
    }
    let dag = reachable_subgraph(&meta.graph, &reach);

    // Seed the env: active inlet values are bound as graph fn params;
    // pre-bound non-branching nodes' outputs are bound by enclosing glue.
    let mut env = Env::default();
    if let LevelSources::Inlets(active) = sources {
        for &i in active {
            let var = Var::Output { node: i, output: 0 };
            env.vals.insert((i, 0), Atom::Var(var));
        }
    }
    for &n in &cx.prebound {
        if cx.branches(n).is_none() {
            let n_outputs = meta.outputs.get(&n).copied().unwrap_or(0);
            for o in 0..n_outputs {
                let var = Var::Output { node: n, output: o };
                env.vals.insert((n, o), Atom::Var(var));
            }
        }
    }

    // Everything to lower: the reachable set minus inlets/outlets (resolved
    // as values, never stepped) and pre-bound non-branching nodes. Pre-bound
    // *branching* nodes stay pending so their dispatch lowers normally.
    let pending: BTreeSet<node::Id> = dag
        .nodes()
        .filter(|n| !meta.inlets.contains(n) && !meta.outlets.contains(n))
        .filter(|n| !cx.prebound.contains(n) || cx.branches(*n).is_some())
        .collect();

    // Delay reads: previous-evaluation values bound before anything runs.
    let mut steps = Vec::new();
    for &d in &meta.delays {
        let consumed =
            dag.contains_node(d) && dag.edges_directed(d, petgraph::Outgoing).next().is_some();
        if consumed {
            let var = Var::Output { node: d, output: 0 };
            env.vals.insert((d, 0), Atom::Var(var));
            steps.push(Step::DelayRead { node: d });
        }
    }

    steps.extend(lower_steps(cx, &dag, pending, &mut env)?);

    // Resolve each outlet's value from the final scope.
    let mut outlets = Vec::with_capacity(meta.outlets.len());
    for &o in &meta.outlets {
        outlets.push(resolve_outlet(&dag, &env, o)?);
    }

    Ok(LevelOut {
        body: Body {
            steps,
            tail: Tail::Ret(vec![]),
        },
        outlets,
    })
}

/// A copy of `g` without the out-edges of delay nodes (preserving all
/// nodes), used for ordering and reachability.
fn strip_delay_out_edges(g: &MetaGraph, delays: &BTreeSet<node::Id>) -> MetaGraph {
    let mut out = MetaGraph::default();
    for n in g.nodes() {
        out.add_node(n);
    }
    for (a, b, w) in g.all_edges() {
        if !delays.contains(&a) {
            out.add_edge(a, b, w.clone());
        }
    }
    out
}

/// The level's static sources: input-less interior nodes (constants), which
/// stay live regardless of which inlets fired.
fn static_sources(meta: &Meta) -> Result<Vec<(node::Id, node::Conns)>, LowerError> {
    use crate::compile::error::{NodeConnsError, TooManyConns};
    let mut sources = Vec::new();
    for n in meta.graph.nodes() {
        if meta.inlets.contains(&n) || meta.outlets.contains(&n) {
            continue;
        }
        if meta
            .graph
            .edges_directed(n, petgraph::Incoming)
            .next()
            .is_some()
        {
            continue;
        }
        let n_out = meta.outputs.get(&n).copied().unwrap_or(0);
        if n_out > 0 {
            let conns = node::Conns::connected(n_out)
                .map_err(|_| NodeConnsError::from(TooManyConns(n_out)))?;
            sources.push((n, conns));
        }
    }
    Ok(sources)
}

/// Resolve outlet `o`'s value: the merged branch export when its sources are
/// conditional, else its single in-scope source.
fn resolve_outlet(dag: &MetaGraph, env: &Env, o: node::Id) -> Result<OutletVal, LowerError> {
    if !dag.contains_node(o) {
        return Ok(OutletVal {
            atom: None,
            conditional: false,
        });
    }
    if let Some(&atom) = env.inputs.get(&(o, 0)) {
        return Ok(OutletVal {
            atom: Some(atom),
            conditional: true,
        });
    }
    let atoms: Vec<Atom> = input_sources(dag, o, 0)
        .into_iter()
        .filter_map(|s| env.vals.get(&s).copied())
        .collect();
    match atoms.len() {
        0 => Ok(OutletVal {
            atom: None,
            conditional: false,
        }),
        1 => Ok(OutletVal {
            atom: Some(atoms[0]),
            conditional: false,
        }),
        _ => Err(LowerError::MixedInputSources { node: o, input: 0 }),
    }
}

/// Collect the node configurations (variants) called anywhere in `body`.
pub(crate) fn collect_confs(body: &Body, confs: &mut BTreeSet<NodeConf>) {
    fn conf(call: &NodeCall) -> NodeConf {
        let inputs = node::Conns::try_from_iter(call.args.iter().map(Option::is_some))
            .expect("arg count exceeds Conns::MAX");
        NodeConf {
            id: call.node,
            conns: NodeConns {
                inputs,
                outputs: call.outputs,
            },
        }
    }
    for step in &body.steps {
        match step {
            Step::Node { call, .. } => {
                confs.insert(conf(call));
            }
            // Delays are intrinsics: no node fn.
            Step::DelayRead { .. } | Step::DelayWrite { .. } => {}
            Step::Join(join) => collect_confs(&join.body, confs),
            Step::Branch { subject, arms, .. } => {
                if let Subject::Call(call) = subject {
                    confs.insert(conf(call));
                }
                for arm in arms {
                    collect_confs(&arm.body, confs);
                }
            }
        }
    }
}

/// Lower every node in `pending` into a sequence of steps, in deterministic
/// dependency order.
fn lower_steps(
    cx: &Cx,
    dag: &MetaGraph,
    mut pending: BTreeSet<node::Id>,
    env: &mut Env,
) -> Result<Vec<Step>, LowerError> {
    let mut steps = Vec::new();
    while let Some(n) = next_node(cx, dag, &pending) {
        pending.remove(&n);
        if cx.meta.delays.contains(&n) {
            // A delay's only step is its write (the read was bound at the
            // top of the level body); an unconnected input writes nothing.
            if let Some(arg) = resolve_input(dag, env, n, 0)? {
                steps.push(Step::DelayWrite { node: n, arg });
            }
        } else if cx.branches(n).is_some() {
            lower_branch(cx, dag, &mut pending, env, n, &mut steps)?;
        } else {
            let call = node_call(cx, dag, env, n)?;
            let dst: Vec<Var> = call
                .outputs
                .iter()
                .enumerate()
                .filter_map(|(o, b)| b.then_some(Var::Output { node: n, output: o }))
                .collect();
            for &var in &dst {
                let Var::Output { node, output } = var else {
                    unreachable!()
                };
                env.vals.insert((node, output), Atom::Var(var));
            }
            steps.push(Step::Node { dst, call });
        }
    }
    Ok(steps)
}

/// The next node to lower: among pending nodes whose in-dag predecessors are
/// all already lowered, the lowest-id non-branch node, else the lowest-id
/// branch (emitting independent work first minimizes branch exports).
fn next_node(cx: &Cx, dag: &MetaGraph, pending: &BTreeSet<node::Id>) -> Option<node::Id> {
    let mut first_branch = None;
    for &n in pending {
        // A pending *delay* predecessor never blocks: consumers read the
        // pre-bound previous value, not the pending write.
        let ready = dag.edges_directed(n, petgraph::Incoming).all(|e_ref| {
            !pending.contains(&e_ref.source()) || cx.meta.delays.contains(&e_ref.source())
        });
        if !ready {
            continue;
        }
        if cx.branches(n).is_none() {
            return Some(n);
        }
        if first_branch.is_none() {
            first_branch = Some(n);
        }
    }
    first_branch
}

/// Build the [`NodeCall`] for `n`, resolving each input from the env. A
/// nested-graph node's call targets its graph fn and always yields all of
/// its outputs.
fn node_call(cx: &Cx, dag: &MetaGraph, env: &Env, n: node::Id) -> Result<NodeCall, LowerError> {
    use crate::compile::error::{NodeConnsError, TooManyConns};
    let meta = cx.meta;
    let n_inputs = meta.inputs.get(&n).copied().unwrap_or(0);
    let mut args = Vec::with_capacity(n_inputs);
    for i in 0..n_inputs {
        args.push(resolve_input(dag, env, n, i)?);
    }
    let graph = cx.nested.contains(&n);
    let outputs = if graph {
        let n_outputs = meta.outputs.get(&n).copied().unwrap_or(0);
        node::Conns::connected(n_outputs)
            .map_err(|_| NodeConnsError::from(TooManyConns(n_outputs)))?
    } else {
        node_outputs(meta, dag, n)?
    };
    Ok(NodeCall {
        node: n,
        args,
        outputs,
        stateful: meta.stateful.contains(&n),
        graph,
    })
}

/// Resolve input `i` of node `n`: the merged override if one was installed
/// (a join param or branch export), else the in-scope source atoms - one
/// directly, several as a `(list ...)` in source order, none as unconnected.
///
/// A source with no in-scope binding is dropped: it lives in a scope that
/// can never be live at the same time as this one (e.g. a sibling branch
/// arm), and its contribution to the same consumer merges in an enclosing
/// scope instead.
fn resolve_input(
    dag: &MetaGraph,
    env: &Env,
    n: node::Id,
    i: usize,
) -> Result<Option<Arg>, LowerError> {
    if let Some(&atom) = env.inputs.get(&(n, i)) {
        return Ok(Some(Arg::One(atom)));
    }
    let atoms: Vec<Atom> = input_sources(dag, n, i)
        .into_iter()
        .filter_map(|s| env.vals.get(&s).copied())
        .collect();
    Ok(match atoms.len() {
        0 => None,
        1 => Some(Arg::One(atoms[0])),
        _ => Some(Arg::List(atoms)),
    })
}

/// The `(source, output)` pairs feeding input `i` of `n`, sorted by source
/// then output for deterministic ordering.
fn input_sources(dag: &MetaGraph, n: node::Id, i: usize) -> Vec<(node::Id, usize)> {
    let mut sources = Vec::new();
    for e_ref in dag.edges_directed(n, petgraph::Incoming) {
        for (edge, _kind) in e_ref.weight() {
            if edge.input.0 as usize == i {
                sources.push((e_ref.source(), edge.output.0 as usize));
            }
        }
    }
    sources.sort();
    sources
}

/// The connected-outputs mask of `n` within the dag.
fn node_outputs(meta: &Meta, dag: &MetaGraph, n: node::Id) -> Result<node::Conns, LowerError> {
    use crate::compile::error::{InvalidOutputIndex, NodeConnsError, TooManyConns};
    let n_outputs = meta.outputs.get(&n).copied().unwrap_or(0);
    let mut outputs = node::Conns::unconnected(n_outputs)
        .map_err(|_| NodeConnsError::from(TooManyConns(n_outputs)))?;
    for e_ref in dag.edges_directed(n, petgraph::Outgoing) {
        for (edge, _kind) in e_ref.weight() {
            let index = edge.output.0 as usize;
            outputs
                .set(index, true)
                .map_err(|_| NodeConnsError::from(InvalidOutputIndex { index, n_outputs }))?;
        }
    }
    Ok(outputs)
}

/// The nodes reachable from `seeds` within `within`, including the seeds.
/// Never expands through a delay node (its value crosses evaluations), nor
/// through a member of `stop` (the member itself is still included).
fn descendants(
    cx: &Cx,
    dag: &MetaGraph,
    seeds: impl IntoIterator<Item = node::Id>,
    within: &BTreeSet<node::Id>,
    stop: &BTreeSet<node::Id>,
) -> BTreeSet<node::Id> {
    let mut reached = BTreeSet::new();
    let mut stack: Vec<node::Id> = seeds.into_iter().filter(|n| within.contains(n)).collect();
    while let Some(n) = stack.pop() {
        if !reached.insert(n) || cx.meta.delays.contains(&n) || stop.contains(&n) {
            continue;
        }
        for e_ref in dag.edges_directed(n, petgraph::Outgoing) {
            let t = e_ref.target();
            if within.contains(&t) && !reached.contains(&t) {
                stack.push(t);
            }
        }
    }
    reached
}

/// The seed targets of one branch arm: targets of `b`'s out-edges whose
/// output the arm mask activates, restricted to `within`.
fn arm_seeds(
    dag: &MetaGraph,
    b: node::Id,
    mask: &node::Conns,
    within: &BTreeSet<node::Id>,
) -> Vec<node::Id> {
    let mut seeds = Vec::new();
    for e_ref in dag.edges_directed(b, petgraph::Outgoing) {
        for (edge, _kind) in e_ref.weight() {
            if mask.get(edge.output.0 as usize).unwrap_or(false) && within.contains(&e_ref.target())
            {
                seeds.push(e_ref.target());
            }
        }
    }
    seeds
}

/// The nodes *unconditionally* evaluated once `seeds` are reached, within
/// `within`: the forward closure that crosses a nested branch only via the
/// nodes every one of its arms unconditionally reaches (an arm reaching
/// nothing - a dead arm that terminates evaluation - blocks the crossing
/// entirely). Unlike [`descendants`], a node reached only through some arms
/// of a nested branch is conditional and excluded.
fn unconditional_reach(
    cx: &Cx,
    dag: &MetaGraph,
    seeds: impl IntoIterator<Item = node::Id>,
    within: &BTreeSet<node::Id>,
) -> BTreeSet<node::Id> {
    let mut reached = BTreeSet::new();
    let mut stack: Vec<node::Id> = seeds.into_iter().filter(|n| within.contains(n)).collect();
    while let Some(n) = stack.pop() {
        if !reached.insert(n) || cx.meta.delays.contains(&n) {
            continue;
        }
        match cx.branches(n) {
            None => {
                for e_ref in dag.edges_directed(n, petgraph::Outgoing) {
                    let t = e_ref.target();
                    if within.contains(&t) && !reached.contains(&t) {
                        stack.push(t);
                    }
                }
            }
            Some(masks) => {
                let mut arms = masks.iter().map(|mask| {
                    unconditional_reach(cx, dag, arm_seeds(dag, n, mask, within), within)
                });
                let mut shared = arms.next().unwrap_or_default();
                for arm in arms {
                    shared = shared.intersection(&arm).copied().collect();
                }
                stack.extend(shared.into_iter().filter(|t| !reached.contains(t)));
            }
        }
    }
    reached
}

/// An export slot bound by a branch dispatch statement: how the join body's
/// final scope yields it, and the atom a bypassing (dead) arm yields instead.
#[derive(Clone, Copy)]
struct Slot {
    ret: SlotRet,
    missing: Atom,
}

/// How a slot's value is read from the join body's final scope (resolved
/// *after* the join lowers, since a cont source may itself be routed through
/// a deeper branch construct's export).
#[derive(Clone, Copy)]
enum SlotRet {
    /// The join param itself.
    Param(Var),
    /// Consumer input `consumer`: a deeper construct's export override when
    /// present, else its single cont `source`'s value.
    Input {
        consumer: (node::Id, usize),
        source: (node::Id, usize),
    },
    /// The cont value in the join's final scope.
    Val((node::Id, usize)),
}

/// Lower branch node `b` and its whole region: arm bodies, the reconvergence
/// join (if any), and the branch statement binding its exports.
fn lower_branch(
    cx: &Cx,
    dag: &MetaGraph,
    pending: &mut BTreeSet<node::Id>,
    env: &mut Env,
    b: node::Id,
    steps: &mut Vec<Step>,
) -> Result<(), LowerError> {
    let meta = cx.meta;
    let arm_masks = cx.branches(b).expect("caller checked branching").clone();
    let subject = if cx.prebound.contains(&b) {
        Subject::PreBound { node: b }
    } else {
        Subject::Call(node_call(cx, dag, env, b)?)
    };

    // Per-arm reach: everything possibly downstream of the arm.
    let no_stop = BTreeSet::new();
    let r_arms: Vec<BTreeSet<node::Id>> = arm_masks
        .iter()
        .map(|mask| descendants(cx, dag, arm_seeds(dag, b, mask, pending), pending, &no_stop))
        .collect();
    let r_all: BTreeSet<node::Id> = r_arms.iter().flatten().copied().collect();
    // An arm is live when it propagates anywhere: into its (pending-local)
    // region, or via an active output straight to a consumer outside the
    // current lowering scope (e.g. an enclosing join's node or an outlet).
    let live: Vec<bool> = arm_masks
        .iter()
        .zip(&r_arms)
        .map(|(mask, r)| {
            if !r.is_empty() {
                return true;
            }
            dag.edges_directed(b, petgraph::Outgoing).any(|e_ref| {
                e_ref
                    .weight()
                    .iter()
                    .any(|(edge, _)| mask.get(edge.output.0 as usize).unwrap_or(false))
            })
        })
        .collect();

    // Reconvergence candidates: nodes every live arm *unconditionally*
    // reaches. Set-reachability is not enough - a node reached only through
    // some arms of a nested branch is conditional and must stay inside that
    // branch's own lowering (the lattice shape).
    let mut live_ucr = arm_masks
        .iter()
        .zip(&live)
        .filter(|&(_, &l)| l)
        .map(|(mask, _)| unconditional_reach(cx, dag, arm_seeds(dag, b, mask, pending), pending));
    let mut cont_cand = live_ucr.next().unwrap_or_default();
    for ucr in live_ucr {
        cont_cand = cont_cand.intersection(&ucr).copied().collect();
    }

    // Nodes in the region that also depend on unevaluated work outside it.
    let ext: BTreeSet<node::Id> = pending
        .iter()
        .copied()
        .filter(|n| !r_all.contains(n) && *n != b)
        .collect();
    let ext_desc = descendants(cx, dag, ext.iter().copied(), pending, &no_stop);
    if let Some(&bad) = r_all
        .iter()
        .find(|n| !cont_cand.contains(n) && ext_desc.contains(n))
    {
        return Err(LowerError::Entangled {
            branch: b,
            node: bad,
        });
    }
    let deferred: BTreeSet<node::Id> = cont_cand.intersection(&ext_desc).copied().collect();

    // Arm regions hold only the work conditional on *this* branch alone:
    // arm reach stops at reconvergence candidates, so nodes that are further
    // conditional on a branch lowered in the join (a cascade) stay out of
    // the arms and join the continuation's pending instead, where the inner
    // branch's own lowering places them.
    let arm_regions: Vec<BTreeSet<node::Id>> = arm_masks
        .iter()
        .map(|mask| {
            descendants(
                cx,
                dag,
                arm_seeds(dag, b, mask, pending),
                pending,
                &cont_cand,
            )
            .difference(&cont_cand)
            .copied()
            .collect()
        })
        .collect();
    let in_armed: BTreeSet<node::Id> = arm_regions.iter().flatten().copied().collect();
    let cont: BTreeSet<node::Id> = r_all
        .iter()
        .copied()
        .filter(|n| !deferred.contains(n) && !in_armed.contains(n))
        .collect();

    // Classify each input of every consumer fed from inside this branch
    // construct. Consumers within an arm resolve lexically inside the arm and
    // `b`'s own inputs were resolved above, so what remains: cont members
    // (lowered in the join; arm-varying inputs become join params) and
    // *outside* consumers - deferred nodes, enclosing-scope nodes, or this
    // level's outlets - whose region-fed inputs flow out as branch exports.
    // Outlet-feeding exports always get a dedicated per-input slot whose
    // missing value is the unfired sentinel.
    let in_arms = |n: node::Id| arm_regions.iter().any(|r| r.contains(&n));
    let mut consumer_inputs: BTreeSet<(node::Id, usize)> = BTreeSet::new();
    for v in cont
        .iter()
        .filter(|v| !cx.meta.delays.contains(v))
        .chain(arm_regions.iter().flatten())
        .chain([&b])
    {
        for e_ref in dag.edges_directed(*v, petgraph::Outgoing) {
            let t = e_ref.target();
            if t == b || in_arms(t) {
                continue;
            }
            for (edge, _kind) in e_ref.weight() {
                consumer_inputs.insert((t, edge.input.0 as usize));
            }
        }
    }
    // (consumer, input) -> param var, for inputs with arm-varying sources.
    let mut params: BTreeMap<(node::Id, usize), Var> = BTreeMap::new();
    // Everything the branch statement binds, in deterministic Var order.
    let mut slots: BTreeMap<Var, Slot> = BTreeMap::new();
    for &(t, i) in &consumer_inputs {
        let sources = input_sources(dag, t, i);
        let arm_s: Vec<(node::Id, usize)> = sources
            .iter()
            .copied()
            .filter(|&(s, _)| s == b || in_arms(s))
            .collect();
        let cont_s: Vec<(node::Id, usize)> = sources
            .iter()
            .copied()
            .filter(|&(s, _)| cont.contains(&s))
            .collect();
        let outlet = meta.outlets.contains(&t);
        let missing = if outlet { Atom::Unfired } else { Atom::Unit };
        let outside = !cont.contains(&t);
        if !arm_s.is_empty() {
            // Arm-varying sources merge into one scalar param; mixing them
            // with simultaneously-alive sources is unsupported. Sources
            // visible in neither the region nor the current scope belong to
            // enclosing scopes (e.g. a sibling outer arm) and merge there.
            let lexical = sources.iter().any(|s| env.vals.contains_key(s));
            if !cont_s.is_empty() || lexical || env.inputs.contains_key(&(t, i)) {
                return Err(LowerError::MixedInputSources { node: t, input: i });
            }
            let param = Var::Input { node: t, input: i };
            params.insert((t, i), param);
            if outside {
                slots.insert(
                    param,
                    Slot {
                        ret: SlotRet::Param(param),
                        missing,
                    },
                );
            }
        } else if outside && !cont_s.is_empty() {
            if outlet {
                // A dedicated slot carrying the outlet's (single) cont
                // source, so a bypassing arm yields the unfired sentinel.
                let lexical = sources.iter().any(|s| env.vals.contains_key(s));
                if cont_s.len() > 1 || lexical || env.inputs.contains_key(&(t, i)) {
                    return Err(LowerError::MixedInputSources { node: t, input: i });
                }
                slots.insert(
                    Var::Input { node: t, input: i },
                    Slot {
                        ret: SlotRet::Input {
                            consumer: (t, i),
                            source: cont_s[0],
                        },
                        missing,
                    },
                );
            } else {
                for (s, o) in cont_s {
                    let var = Var::Output { node: s, output: o };
                    slots.insert(
                        var,
                        Slot {
                            ret: SlotRet::Val((s, o)),
                            missing: Atom::Unit,
                        },
                    );
                }
            }
        }
    }

    // The join body: the cont nodes, with arm-varying inputs reading their
    // params, ending by yielding the export slots' values.
    let export_vars: Vec<Var> = slots.keys().copied().collect();
    let join_id = cont.first().copied().unwrap_or(b);
    let join = if !cont.is_empty() || !slots.is_empty() {
        let mut join_env = env.clone();
        let mut param_vars: Vec<Var> = Vec::new();
        for (&(n, i), &param) in &params {
            join_env.inputs.insert((n, i), Atom::Var(param));
            param_vars.push(param);
        }
        let join_steps = lower_steps(cx, dag, cont.clone(), &mut join_env)?;
        let ret = slots
            .values()
            .map(|slot| match slot.ret {
                SlotRet::Param(p) => Atom::Var(p),
                SlotRet::Input { consumer, source } => join_env
                    .inputs
                    .get(&consumer)
                    .or(join_env.vals.get(&source))
                    .copied()
                    .unwrap_or(slot.missing),
                SlotRet::Val(source) => join_env.vals.get(&source).copied().unwrap_or(slot.missing),
            })
            .collect();
        Some(Join {
            id: join_id,
            params: param_vars,
            rec: false,
            body: Body {
                steps: join_steps,
                tail: Tail::Ret(ret),
            },
        })
    } else {
        None
    };

    // Arm bodies. Live arms jump to the join (when one exists) passing each
    // param's value as produced by that arm; dead arms yield every slot's
    // missing value directly, bypassing the join.
    let mut arms = Vec::with_capacity(arm_masks.len());
    for (k, mask) in arm_masks.iter().enumerate() {
        let mut arm_env = env.clone();
        let binds: Vec<Var> = mask
            .iter()
            .enumerate()
            .filter_map(|(o, active)| active.then_some(Var::Output { node: b, output: o }))
            .collect();
        for &var in &binds {
            let Var::Output { node, output } = var else {
                unreachable!()
            };
            arm_env.vals.insert((node, output), Atom::Var(var));
        }
        let arm_steps = lower_steps(cx, dag, arm_regions[k].clone(), &mut arm_env)?;
        let tail = if live[k] && join.is_some() {
            let mut args = Vec::with_capacity(params.len());
            for &(n, i) in params.keys() {
                let missing = if meta.outlets.contains(&n) {
                    Atom::Unfired
                } else {
                    Atom::Unit
                };
                args.push(arm_param_arg(
                    dag,
                    &arm_env,
                    &arm_regions[k],
                    b,
                    mask,
                    n,
                    i,
                    missing,
                )?);
            }
            Tail::Jump {
                join: join_id,
                args,
            }
        } else {
            Tail::Ret(slots.values().map(|slot| slot.missing).collect())
        };
        arms.push(Arm {
            ix: k,
            binds,
            body: Body {
                steps: arm_steps,
                tail,
            },
        });
    }

    // Consume the region; deferred nodes stay pending and read the exports.
    for n in r_all.iter() {
        if !deferred.contains(n) {
            pending.remove(n);
        }
    }
    for &var in &export_vars {
        match var {
            Var::Output { node, output } => {
                env.vals.insert((node, output), Atom::Var(var));
            }
            Var::Input { node, input } => {
                env.inputs.insert((node, input), Atom::Var(var));
            }
            Var::Result { .. } => unreachable!("exports are output or input vars"),
        }
    }

    steps.extend(join.map(Step::Join));
    steps.push(Step::Branch {
        subject,
        dst: export_vars,
        arms,
    });
    Ok(())
}

/// The atom arm `k` passes for the join param merging input `(n, i)`: the
/// value of the arm-local source feeding it, or `missing` when this arm
/// produces none.
#[allow(clippy::too_many_arguments)]
fn arm_param_arg(
    dag: &MetaGraph,
    arm_env: &Env,
    arm_region: &BTreeSet<node::Id>,
    b: node::Id,
    mask: &node::Conns,
    n: node::Id,
    i: usize,
    missing: Atom,
) -> Result<Atom, LowerError> {
    // An inner branch within this arm may already have merged the input's
    // in-arm sources into an export of its own; pass that through. A further
    // direct alive source alongside it would need a second scalar slot.
    if let Some(&atom) = arm_env.inputs.get(&(n, i)) {
        let direct = input_sources(dag, n, i)
            .into_iter()
            .any(|s| arm_env.vals.contains_key(&s));
        if direct {
            return Err(LowerError::MixedInputSources { node: n, input: i });
        }
        return Ok(atom);
    }
    let mut atoms = Vec::new();
    for (src, out) in input_sources(dag, n, i) {
        let alive = if src == b {
            mask.get(out).unwrap_or(false)
        } else {
            arm_region.contains(&src)
        };
        if !alive {
            continue;
        }
        let &atom = arm_env
            .vals
            .get(&(src, out))
            .ok_or(LowerError::Unresolved {
                node: src,
                output: out,
                consumer: n,
            })?;
        atoms.push(atom);
    }
    match atoms.len() {
        0 => Ok(missing),
        1 => Ok(atoms[0]),
        _ => Err(LowerError::MixedInputSources { node: n, input: i }),
    }
}
