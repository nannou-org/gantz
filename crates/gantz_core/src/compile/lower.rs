//! Lowering one graph level (via its [`Meta`]) to an IR [`Body`].
//!
//! Nodes are emitted as steps in a deterministic dependency order (ready
//! nodes by ascending id, branches last so as much independent work as
//! possible precedes each dispatch). At a branch:
//!
//! - each arm's region (nodes reachable only while that arm is live) lowers
//!   into the arm's body, recursively;
//! - nodes reached by *every* live arm reconverge: they lower once into a
//!   join point, and each consumed input whose sources are arm-varying
//!   becomes a join parameter (arms pass their value, or `'()` when they
//!   don't produce one);
//! - a reconvergent node that also depends on unevaluated work *outside* the
//!   branch region is deferred: it stays pending after the branch statement,
//!   and the values it needs from inside flow out as branch exports (the
//!   join returns them, the branch statement binds them). This subsumes the
//!   old cross-component root ordering.
//!
//! Dead arms (reaching nothing) yield `'()` for every export and bypass the
//! join, so reconvergent work runs only when a live arm actually jumps.

use crate::{
    compile::{
        Meta, MetaGraph,
        error::LowerError,
        flow::{NodeConf, NodeConns, reachable_subgraph},
        ir::{Arg, Arm, Atom, Body, Join, NodeCall, Step, Tail, Var},
    },
    node,
};
use petgraph::visit::EdgeRef;
use std::collections::{BTreeMap, BTreeSet, HashSet};

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

/// Lower one level's evaluation to a [`Body`] yielding no values.
///
/// `push`/`pull` are the source nodes with their participating connections
/// (resolved at this level). Inlet/outlet nodes are excluded: at the root
/// they are inert (outlet values are never read, inlet values never bound).
pub(crate) fn entry_body(
    meta: &Meta,
    push: Vec<(node::Id, node::Conns)>,
    pull: Vec<(node::Id, node::Conns)>,
) -> Result<Body, LowerError> {
    let reach: HashSet<node::Id> = super::eval_order(&meta.graph, push, pull)
        .filter(|n| !meta.inlets.contains(n) && !meta.outlets.contains(n))
        .collect();
    let dag = reachable_subgraph(&meta.graph, &reach);
    let pending: BTreeSet<node::Id> = dag.nodes().collect();
    let mut env = Env::default();
    let steps = lower_steps(meta, &dag, pending, &mut env)?;
    Ok(Body {
        steps,
        tail: Tail::Ret(vec![]),
    })
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
            Step::Join(join) => collect_confs(&join.body, confs),
            Step::Branch { call, arms, .. } => {
                confs.insert(conf(call));
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
    meta: &Meta,
    dag: &MetaGraph,
    mut pending: BTreeSet<node::Id>,
    env: &mut Env,
) -> Result<Vec<Step>, LowerError> {
    let mut steps = Vec::new();
    while let Some(n) = next_node(meta, dag, &pending) {
        pending.remove(&n);
        if meta.branches.contains_key(&n) {
            lower_branch(meta, dag, &mut pending, env, n, &mut steps)?;
        } else {
            let call = node_call(meta, dag, env, n)?;
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
fn next_node(meta: &Meta, dag: &MetaGraph, pending: &BTreeSet<node::Id>) -> Option<node::Id> {
    let mut first_branch = None;
    for &n in pending {
        let ready = dag
            .edges_directed(n, petgraph::Incoming)
            .all(|e_ref| !pending.contains(&e_ref.source()));
        if !ready {
            continue;
        }
        if !meta.branches.contains_key(&n) {
            return Some(n);
        }
        if first_branch.is_none() {
            first_branch = Some(n);
        }
    }
    first_branch
}

/// Build the [`NodeCall`] for `n`, resolving each input from the env.
fn node_call(meta: &Meta, dag: &MetaGraph, env: &Env, n: node::Id) -> Result<NodeCall, LowerError> {
    let n_inputs = meta.inputs.get(&n).copied().unwrap_or(0);
    let mut args = Vec::with_capacity(n_inputs);
    for i in 0..n_inputs {
        args.push(resolve_input(dag, env, n, i)?);
    }
    Ok(NodeCall {
        node: n,
        args,
        outputs: node_outputs(meta, dag, n)?,
        stateful: meta.stateful.contains(&n),
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
fn descendants(
    dag: &MetaGraph,
    seeds: impl IntoIterator<Item = node::Id>,
    within: &BTreeSet<node::Id>,
) -> BTreeSet<node::Id> {
    let mut reached = BTreeSet::new();
    let mut stack: Vec<node::Id> = seeds.into_iter().filter(|n| within.contains(n)).collect();
    while let Some(n) = stack.pop() {
        if !reached.insert(n) {
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
    meta: &Meta,
    dag: &MetaGraph,
    seeds: impl IntoIterator<Item = node::Id>,
    within: &BTreeSet<node::Id>,
) -> BTreeSet<node::Id> {
    let mut reached = BTreeSet::new();
    let mut stack: Vec<node::Id> = seeds.into_iter().filter(|n| within.contains(n)).collect();
    while let Some(n) = stack.pop() {
        if !reached.insert(n) {
            continue;
        }
        match meta.branches.get(&n) {
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
                    unconditional_reach(meta, dag, arm_seeds(dag, n, mask, within), within)
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

/// Lower branch node `b` and its whole region: arm bodies, the reconvergence
/// join (if any), and the branch statement binding its exports.
fn lower_branch(
    meta: &Meta,
    dag: &MetaGraph,
    pending: &mut BTreeSet<node::Id>,
    env: &mut Env,
    b: node::Id,
    steps: &mut Vec<Step>,
) -> Result<(), LowerError> {
    let arm_masks = &meta.branches[&b];
    let call = node_call(meta, dag, env, b)?;

    // Per-arm regions: nodes reachable while that arm is live.
    let r_arms: Vec<BTreeSet<node::Id>> = arm_masks
        .iter()
        .map(|mask| descendants(dag, arm_seeds(dag, b, mask, pending), pending))
        .collect();
    let r_all: BTreeSet<node::Id> = r_arms.iter().flatten().copied().collect();
    // An arm is live when it propagates anywhere: into its (pending-local)
    // region, or via an active output straight to a consumer outside the
    // current lowering scope (e.g. an enclosing join's node).
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
        .map(|(mask, _)| unconditional_reach(meta, dag, arm_seeds(dag, b, mask, pending), pending));
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
    let ext_desc = descendants(dag, ext.iter().copied(), pending);
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
    let cont: BTreeSet<node::Id> = cont_cand.difference(&deferred).copied().collect();
    let arm_regions: Vec<BTreeSet<node::Id>> = r_arms
        .iter()
        .map(|r| r.difference(&cont_cand).copied().collect())
        .collect();

    // Classify each input of every consumer fed from inside this branch
    // construct. Consumers within an arm resolve lexically inside the arm and
    // `b`'s own inputs were resolved above, so what remains: cont members
    // (lowered in the join; arm-varying inputs become join params) and
    // *outside* consumers - deferred nodes or enclosing-scope nodes - whose
    // region-fed inputs flow out as branch exports (arm-varying ones via a
    // param the join returns, cont values under their own names).
    let in_arms = |n: node::Id| arm_regions.iter().any(|r| r.contains(&n));
    let mut consumer_inputs: BTreeSet<(node::Id, usize)> = BTreeSet::new();
    for v in cont.iter().chain(arm_regions.iter().flatten()).chain([&b]) {
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
    let mut exports: BTreeSet<Var> = BTreeSet::new();
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
                exports.insert(param);
            }
        } else if outside {
            for (s, o) in cont_s {
                exports.insert(Var::Output { node: s, output: o });
            }
        }
    }

    // The join body: the cont nodes, with arm-varying inputs reading their
    // params, ending by yielding the exports.
    let export_vars: Vec<Var> = exports.iter().copied().collect();
    let join_id = cont.first().copied().unwrap_or(b);
    let join = if !cont.is_empty() || !exports.is_empty() {
        let mut join_env = env.clone();
        let mut param_vars: Vec<Var> = Vec::new();
        for (&(n, i), &param) in &params {
            join_env.inputs.insert((n, i), Atom::Var(param));
            param_vars.push(param);
        }
        let join_steps = lower_steps(meta, dag, cont.clone(), &mut join_env)?;
        let ret = export_vars.iter().map(|&v| Atom::Var(v)).collect();
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
    // param's value as produced by that arm ('() when it doesn't); dead arms
    // yield '() for every export directly, bypassing the join.
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
        let arm_steps = lower_steps(meta, dag, arm_regions[k].clone(), &mut arm_env)?;
        let tail = if live[k] && join.is_some() {
            let mut args = Vec::with_capacity(params.len());
            for &(n, i) in params.keys() {
                args.push(arm_param_arg(
                    dag,
                    &arm_env,
                    &arm_regions[k],
                    b,
                    mask,
                    n,
                    i,
                )?);
            }
            Tail::Jump {
                join: join_id,
                args,
            }
        } else {
            Tail::Ret(vec![Atom::Unit; export_vars.len()])
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
        }
    }

    steps.extend(join.map(Step::Join));
    steps.push(Step::Branch {
        call,
        dst: export_vars,
        arms,
    });
    Ok(())
}

/// The atom arm `k` passes for the join param merging input `(n, i)`: the
/// value of the arm-local source feeding it, or `'()` when this arm produces
/// none.
fn arm_param_arg(
    dag: &MetaGraph,
    arm_env: &Env,
    arm_region: &BTreeSet<node::Id>,
    b: node::Id,
    mask: &node::Conns,
    n: node::Id,
    i: usize,
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
        0 => Ok(Atom::Unit),
        1 => Ok(atoms[0]),
        _ => Err(LowerError::MixedInputSources { node: n, input: i }),
    }
}
