//! Lowering one graph level, via its [`Meta`], to an IR [`Body`].
//!
//! Nodes are emitted as steps in a deterministic dependency order. Ready
//! nodes go by ascending id. Branches go last so as much independent work
//! as possible precedes each dispatch. At a branch:
//!
//! - Each arm's region lowers into the arm's body, recursively. The region
//!   is the nodes reachable only while that arm is live.
//! - Nodes every live arm unconditionally reaches reconverge. They lower
//!   once into a join point. Each consumed input whose sources are
//!   arm-varying becomes a join parameter. Arms pass their value, or `'()`
//!   when they do not produce one.
//! - Values consumed outside the branch construct flow out as branch
//!   exports. The consumers are deferred nodes, enclosing scopes, or this
//!   level's outlets. The join returns the exports and the dispatch
//!   statement binds them.
//!
//! A dead arm reaches nothing. It yields the missing value for every export
//! and bypasses the join. The missing value is `'()`. For outlet-feeding
//! exports it is the unfired sentinel, so a level result can distinguish
//! "did not fire" from "fired with `'()`". Reconvergent work thus runs only
//! when a live arm jumps.
//!
//! A node in the region that also reads a value fired before the branch runs
//! whichever arm the branch takes. Unless every arm reaches it already, it
//! moves out of the region with the work downstream of it. The moved nodes
//! lower after the branch statement. Each source they read from the region
//! exports alone, and is the unfired sentinel on a path that did not produce
//! it, also through a branch nested in the region. An input reading such an
//! export is an [`Arg::Maybe`]. It gets the sources that fired, and the call
//! picks its variant from them.
//!
//! A level is lowered the same way whether entered from an entrypoint
//! ([`LevelSources::Eval`]) or as a graph fn ([`LevelSources::Inlets`]).
//! Inlets resolve as pre-bound parameter values. Outlet values are returned
//! to the caller via [`LevelOut`].

use crate::{
    compile::{
        Meta, MetaGraph,
        error::LowerError,
        ir::{Arg, Arm, Atom, Body, Join, NodeCall, Step, Subject, Tail, Var},
    },
    node,
};
use petgraph::visit::EdgeRef;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;

/// A node variant within an evaluation: its identity and connectedness.
///
/// Maps directly to a generated node fn.
#[derive(Copy, Clone, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub(crate) struct NodeConf {
    pub id: node::Id,
    pub conns: NodeConns,
}

/// The connectedness of a node for a particular evaluation step.
#[derive(Copy, Clone, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub(crate) struct NodeConns {
    /// The active inputs.
    pub inputs: node::Conns,
    /// Includes all connected outputs, conditional or not.
    pub outputs: node::Conns,
}

impl fmt::Debug for NodeConf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({:?}: {:?})", self.id, self.conns)
    }
}

impl fmt::Debug for NodeConns {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "([{}], [{}])", self.inputs, self.outputs)
    }
}

/// The fixed context for lowering one level.
pub(crate) struct Cx<'a> {
    pub meta: &'a Meta,
    /// Additional per-entrypoint branch masks atop `meta.branches`. These are
    /// bridged children whose inner push reaches their outlets through
    /// branching.
    pub extra_branches: BTreeMap<node::Id, Vec<node::Conns>>,
    /// Nodes already evaluated by enclosing glue. Their [`Var::Result`] pair
    /// or outputs are bound before the body runs.
    pub prebound: BTreeSet<node::Id>,
}

/// What drives a level's evaluation.
pub(crate) enum LevelSources {
    /// A graph-fn variant with these inlets active. All-active also pulls
    /// from the outlets. A subset pushes from the active inlets plus the
    /// level's static sources only.
    Inlets(BTreeSet<node::Id>),
    /// Entrypoint sources resolved at this level. Bridged children are
    /// included as pre-evaluated nodes pushing over their produced outputs.
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
    /// Whether the value may be the unfired sentinel at runtime because it
    /// flowed through a branch export.
    pub conditional: bool,
}

/// A lowered level. Its body and the values of its outlets in id order.
pub(crate) struct LevelOut {
    pub body: Body,
    pub outlets: Vec<OutletVal>,
}

/// The lexical environment during lowering. It records which atom currently
/// holds each value. It also records per-input overrides where arm-varying
/// sources were merged into a single join parameter or export.
#[derive(Clone, Default)]
struct Env {
    /// Maps `(node, output)` to the in-scope atom holding that value.
    vals: BTreeMap<(node::Id, usize), Atom>,
    /// Maps `(node, input)` to the atom standing in for all of that input's
    /// sources.
    inputs: BTreeMap<(node::Id, usize), Atom>,
    /// The values in `vals` that fired on every path to this point. A branch
    /// export may be `'()` from an arm that did not produce it. A delay read
    /// fires nothing. Neither is in the set.
    fired: BTreeSet<(node::Id, usize)>,
    /// Nodes moved out of the region of an enclosing branch. Each source they
    /// read from a branch exports alone, with the unfired sentinel when it did
    /// not fire.
    sentinel: BTreeSet<node::Id>,
    /// The export vars in `vals` that may be the unfired sentinel. An input
    /// reading one resolves to an [`Arg::Maybe`].
    maybe: BTreeSet<Var>,
}

impl<'a> Cx<'a> {
    /// The branch arm masks of `n`, when branching. Entrypoint-specific
    /// bridged branches take precedence over the graph-wide set.
    fn branches(&self, n: node::Id) -> Option<&Vec<node::Conns>> {
        self.extra_branches.get(&n).or(self.meta.branches.get(&n))
    }
}

/// The reach dag for one level's evaluation. The subgraph of nodes the given
/// sources evaluate, with all edges between them.
///
/// Evaluation never propagates through a delay. Its value crosses between
/// evaluations. Reachability therefore runs on a graph with delay out-edges
/// stripped. This is also what legalizes cycles passing through one. A delay
/// whose stored value is consumed by a reached node joins the reach so its
/// read binding exists. Shared by [`level_body`] and the outlet-activation
/// analysis so both see identical reachability.
pub(crate) fn level_reach_dag(
    meta: &Meta,
    sources: &LevelSources,
) -> Result<MetaGraph, LowerError> {
    let conn1 = || node::Conns::connected(1).unwrap();
    let (push, pull) = match sources {
        LevelSources::Inlets(active) => {
            let mut push: Vec<(node::Id, node::Conns)> =
                active.iter().map(|&n| (n, conn1())).collect();
            if active.len() == meta.inlets.len() {
                // All inlets active. Inlets push and outlets pull.
                let pull = meta.outlets.iter().map(|&n| (n, conn1())).collect();
                (push, pull)
            } else {
                // Subset. Static sources stay live. No outlet pull, so an
                // unfired inlet's exclusive subtree is excluded.
                push.extend(static_sources(meta)?);
                (push, vec![])
            }
        }
        LevelSources::Eval { push, pull } => (push.clone(), pull.clone()),
    };

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
    Ok(reachable_subgraph(&meta.graph, &reach))
}

/// Filter unreachable nodes from the given metagraph.
fn reachable_subgraph(g: &MetaGraph, reachable: &HashSet<node::Id>) -> MetaGraph {
    g.all_edges()
        .filter(|(a, b, _)| reachable.contains(a) && reachable.contains(b))
        .map(|(a, b, w)| (a, b, w.clone()))
        .collect()
}

/// Lower one level's evaluation.
pub(crate) fn level_body(cx: &Cx, sources: &LevelSources) -> Result<LevelOut, LowerError> {
    let meta = cx.meta;
    let dag = level_reach_dag(meta, sources)?;

    // Seed the env. Active inlet values are bound as graph fn params.
    // Enclosing glue binds the outputs of pre-bound non-branching nodes.
    let mut env = Env::default();
    if let LevelSources::Inlets(active) = sources {
        let vars: Vec<Var> = active
            .iter()
            .map(|&i| Var::Output { node: i, output: 0 })
            .collect();
        bind_fired(&mut env, &vars);
    }
    for &n in &cx.prebound {
        if cx.branches(n).is_none() {
            let n_outputs = meta.outputs.get(&n).copied().unwrap_or(0);
            let vars: Vec<Var> = (0..n_outputs)
                .map(|o| Var::Output { node: n, output: o })
                .collect();
            bind_fired(&mut env, &vars);
        }
    }

    // Everything to lower. The reachable set minus inlets, outlets and
    // pre-bound non-branching nodes. Inlets and outlets resolve as values and
    // are never stepped. Pre-bound branching nodes stay pending so their
    // dispatch lowers normally.
    let pending: BTreeSet<node::Id> = dag
        .nodes()
        .filter(|n| !meta.inlets.contains(n) && !meta.outlets.contains(n))
        .filter(|n| !cx.prebound.contains(n) || cx.branches(*n).is_some())
        .collect();

    // Delay reads. Previous-evaluation values are bound before anything runs.
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

/// A copy of `g` without the out-edges of delay nodes. All nodes are kept.
/// Used for ordering and reachability.
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

/// The level's static sources. These are input-less interior nodes such as
/// constants. They stay live regardless of which inlets fired.
fn static_sources(meta: &Meta) -> Result<Vec<(node::Id, node::Conns)>, LowerError> {
    use crate::compile::error::TooManyConns;
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
            let conns = node::Conns::connected(n_out).map_err(|_| LowerError::Conns {
                node: Some(n),
                error: TooManyConns(n_out).into(),
            })?;
            sources.push((n, conns));
        }
    }
    Ok(sources)
}

/// Resolve outlet `o`'s value. It is the merged branch export when its
/// sources are conditional, else its single in-scope source.
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

/// Collect the node variants called anywhere in `body`.
pub(crate) fn collect_confs(body: &Body, confs: &mut BTreeSet<NodeConf>) {
    // The variants a call may pick. An `Arg::Maybe` input with only sources
    // that may not fire is unconnected in some of them.
    fn call_confs(call: &NodeCall) -> Vec<NodeConf> {
        let mut masks: Vec<Vec<bool>> = vec![vec![]];
        for arg in &call.args {
            let connected: &[bool] = match arg {
                None => &[false],
                Some(Arg::Maybe(srcs)) if srcs.iter().all(|&(_, maybe)| maybe) => &[false, true],
                Some(Arg::One(_) | Arg::List(_) | Arg::Maybe(_)) => &[true],
            };
            masks = masks
                .iter()
                .flat_map(|mask| connected.iter().map(|&c| [&mask[..], &[c]].concat()))
                .collect();
        }
        masks
            .into_iter()
            .map(|mask| NodeConf {
                id: call.node,
                conns: NodeConns {
                    inputs: node::Conns::try_from_iter(mask).expect("arg count exceeds Conns::MAX"),
                    outputs: call.outputs,
                },
            })
            .collect()
    }
    for step in &body.steps {
        match step {
            Step::Node { call, .. } => {
                confs.extend(call_confs(call));
            }
            // Delays are intrinsics and have no node fn.
            Step::DelayRead { .. } | Step::DelayWrite { .. } => {}
            Step::Join(join) => collect_confs(&join.body, confs),
            Step::Branch { subject, arms, .. } => {
                if let Subject::Call(call) = subject {
                    confs.extend(call_confs(call));
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
            // A delay's only step is its write. The read was bound at the top
            // of the level body. An unconnected input writes nothing.
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
            bind_fired(env, &dst);
            steps.push(Step::Node { dst, call });
        }
    }
    Ok(steps)
}

/// Bind each of the output `vars` in `env` as a fired value.
fn bind_fired(env: &mut Env, vars: &[Var]) {
    for &var in vars {
        let Var::Output { node, output } = var else {
            unreachable!("only node outputs fire")
        };
        env.vals.insert((node, output), Atom::Var(var));
        env.fired.insert((node, output));
    }
}

/// The next node to lower. Among pending nodes whose in-dag predecessors are
/// all lowered, pick the lowest-id non-branch node, else the lowest-id
/// branch. Emitting independent work first minimizes branch exports.
fn next_node(cx: &Cx, dag: &MetaGraph, pending: &BTreeSet<node::Id>) -> Option<node::Id> {
    let mut first_branch = None;
    for &n in pending {
        // A pending delay predecessor never blocks. Consumers read the
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

/// Build the [`NodeCall`] for `n`, resolving each input from the env.
fn node_call(cx: &Cx, dag: &MetaGraph, env: &Env, n: node::Id) -> Result<NodeCall, LowerError> {
    let meta = cx.meta;
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

/// Resolve input `i` of node `n`. Use the merged override if one was
/// installed, that is a join param or branch export. Else use the in-scope
/// source atoms. One source passes directly. Several pass as a `(list ...)`
/// in source order. None means unconnected. When a source atom may be the
/// unfired sentinel, they pass as an [`Arg::Maybe`] instead.
///
/// A source with no in-scope binding is dropped. It lives in a scope that
/// can never be live at the same time as this one, for example a sibling
/// branch arm. Its contribution to the same consumer merges in an enclosing
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
    let srcs: Vec<(Atom, bool)> = input_sources(dag, n, i)
        .into_iter()
        .filter_map(|s| env.vals.get(&s).copied())
        .map(|atom| (atom, matches!(atom, Atom::Var(v) if env.maybe.contains(&v))))
        .collect();
    if srcs.iter().any(|&(_, maybe)| maybe) {
        return Ok(Some(Arg::Maybe(srcs)));
    }
    let atoms: Vec<Atom> = srcs.into_iter().map(|(atom, _)| atom).collect();
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
    let conns_err = |error: NodeConnsError| LowerError::Conns {
        node: Some(n),
        error,
    };
    let n_outputs = meta.outputs.get(&n).copied().unwrap_or(0);
    let mut outputs = node::Conns::unconnected(n_outputs)
        .map_err(|_| conns_err(TooManyConns(n_outputs).into()))?;
    for e_ref in dag.edges_directed(n, petgraph::Outgoing) {
        for (edge, _kind) in e_ref.weight() {
            let index = edge.output.0 as usize;
            outputs
                .set(index, true)
                .map_err(|_| conns_err(InvalidOutputIndex { index, n_outputs }.into()))?;
        }
    }
    Ok(outputs)
}

/// The nodes reachable from `seeds` within `within`, including the seeds.
/// Never expands through a delay node, since its value crosses evaluations.
/// Never expands through a member of `stop`, but the member itself is still
/// included.
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

/// The nodes unconditionally evaluated once `seeds` are reached, within
/// `within`. This is the forward closure that crosses a nested branch only
/// via the nodes every one of its arms unconditionally reaches. A dead arm
/// reaches nothing and blocks the crossing entirely. Unlike
/// [`descendants`], a node reached only through some arms of a nested
/// branch is conditional and excluded.
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

/// The nodes in the region of branch `b` that also read a value fired before
/// the branch. Each must run whichever arm the branch takes. A node every arm
/// reaches already is left out. So is a node that waits on unevaluated work
/// outside the region.
fn fed_nodes(
    cx: &Cx,
    dag: &MetaGraph,
    env: &Env,
    b: node::Id,
    pending: &BTreeSet<node::Id>,
    r_all: &BTreeSet<node::Id>,
    ext_desc: &BTreeSet<node::Id>,
) -> BTreeSet<node::Id> {
    let fed: BTreeSet<node::Id> = r_all
        .iter()
        .copied()
        .filter(|&n| !ext_desc.contains(&n) && fired_input(dag, env, n))
        .collect();
    if fed.is_empty() {
        return fed;
    }
    let arm_masks = cx.branches(b).expect("caller checked branching");
    let mut reach = arm_masks
        .iter()
        .map(|mask| unconditional_reach(cx, dag, arm_seeds(dag, b, mask, pending), pending));
    let first = reach.next().unwrap_or_default();
    let every_arm = reach.fold(first, |acc, r| acc.intersection(&r).copied().collect());
    fed.difference(&every_arm).copied().collect()
}

/// Whether an input of `n` reads a value that fired on every path to this
/// point.
fn fired_input(dag: &MetaGraph, env: &Env, n: node::Id) -> bool {
    dag.edges_directed(n, petgraph::Incoming).any(|e_ref| {
        e_ref.weight().iter().any(|(edge, _)| {
            let src = (e_ref.source(), edge.output.0 as usize);
            !env.inputs.contains_key(&(n, edge.input.0 as usize)) && env.fired.contains(&src)
        })
    })
}

/// An export slot bound by a branch dispatch statement. It records how the
/// join body's final scope yields it, and the atom a dead arm yields instead.
#[derive(Clone, Copy)]
struct Slot {
    ret: SlotRet,
    missing: Atom,
}

/// How a slot's value is read from the join body's final scope. It is
/// resolved after the join lowers, since a cont source may itself be routed
/// through a deeper branch construct's export.
#[derive(Clone, Copy)]
enum SlotRet {
    /// The join param itself.
    Param(Var),
    /// Consumer input `consumer`. A deeper construct's export override when
    /// present, else its single cont `source`'s value.
    Input {
        consumer: (node::Id, usize),
        source: (node::Id, usize),
    },
    /// The cont value in the join's final scope.
    Val((node::Id, usize)),
}

/// Lower branch node `b` and its whole region. That is the arm bodies, the
/// reconvergence join if any, and the branch statement binding its exports.
/// The nodes that run on every arm move out of the region. See
/// [`fed_nodes`].
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

    // Per-arm reach. Everything possibly downstream of the arm.
    let no_stop = BTreeSet::new();
    let r_arms: Vec<BTreeSet<node::Id>> = arm_masks
        .iter()
        .map(|mask| descendants(cx, dag, arm_seeds(dag, b, mask, pending), pending, &no_stop))
        .collect();
    let r_all: BTreeSet<node::Id> = r_arms.iter().flatten().copied().collect();

    // Nodes in the region that also depend on unevaluated work outside it.
    let ext = pending
        .iter()
        .copied()
        .filter(|n| !r_all.contains(n) && *n != b);
    let ext_desc = descendants(cx, dag, ext, pending, &no_stop);

    // Nodes fed by a value fired before the branch move out of the region,
    // with the work downstream of them. They lower after the branch
    // statement and read its exports.
    let fed = fed_nodes(cx, dag, env, b, pending, &r_all, &ext_desc);
    let moved = descendants(cx, dag, fed.iter().copied(), pending, &no_stop);
    let region: BTreeSet<node::Id> = r_all.difference(&moved).copied().collect();
    let r_arms: Vec<BTreeSet<node::Id>> = r_arms
        .iter()
        .map(|r| r.intersection(&region).copied().collect())
        .collect();
    // The consumers that read each source exported alone, with the unfired
    // sentinel when it did not fire. These are the moved nodes of this and
    // every enclosing branch.
    let sentinel = |t: node::Id| moved.contains(&t) || env.sentinel.contains(&t);

    // An arm is live when it propagates anywhere. That is into its
    // pending-local region, or via an active output straight to a consumer
    // outside the current lowering scope, for example an outlet.
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

    // Reconvergence candidates. Nodes every live arm unconditionally reaches.
    // Set-reachability is not enough. A node reached only through some arms
    // of a nested branch is conditional and must stay inside that branch's
    // own lowering.
    let mut live_ucr = arm_masks
        .iter()
        .zip(&live)
        .filter(|&(_, &l)| l)
        .map(|(mask, _)| unconditional_reach(cx, dag, arm_seeds(dag, b, mask, &region), &region));
    let mut cont_cand = live_ucr.next().unwrap_or_default();
    for ucr in live_ucr {
        cont_cand = cont_cand.intersection(&ucr).copied().collect();
    }

    if let Some(&bad) = region
        .iter()
        .find(|n| !cont_cand.contains(n) && ext_desc.contains(n))
    {
        return Err(LowerError::Entangled {
            branch: b,
            node: bad,
        });
    }
    let deferred: BTreeSet<node::Id> = cont_cand.intersection(&ext_desc).copied().collect();

    // Arm regions hold only the work conditional on this branch alone. Arm
    // reach stops at reconvergence candidates. Nodes that are further
    // conditional on a branch lowered in the join therefore stay out of the
    // arms. They join the continuation's pending instead, where the inner
    // branch's own lowering places them.
    let arm_regions: Vec<BTreeSet<node::Id>> = arm_masks
        .iter()
        .map(|mask| {
            descendants(
                cx,
                dag,
                arm_seeds(dag, b, mask, &region),
                &region,
                &cont_cand,
            )
            .difference(&cont_cand)
            .copied()
            .collect()
        })
        .collect();
    let in_armed: BTreeSet<node::Id> = arm_regions.iter().flatten().copied().collect();
    let cont: BTreeSet<node::Id> = region
        .iter()
        .copied()
        .filter(|n| !deferred.contains(n) && !in_armed.contains(n))
        .collect();

    // Classify each input of every consumer fed from inside this branch
    // construct. Consumers within an arm resolve lexically inside the arm.
    // `b`'s own inputs were resolved above. What remains are cont members
    // and outside consumers. Cont members lower in the join and their
    // arm-varying inputs become join params. Outside consumers are deferred
    // nodes, enclosing-scope nodes, or this level's outlets. Their
    // region-fed inputs flow out as branch exports. Outlet-feeding exports
    // always get a dedicated per-input slot whose missing value is the
    // unfired sentinel.
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
    // Maps `(consumer, input)` to its param var, for inputs with arm-varying
    // sources.
    let mut params: BTreeMap<(node::Id, usize), Var> = BTreeMap::new();
    // The arm-varying sources exported alone. Each is a join param named
    // after the source.
    let mut src_params: BTreeSet<(node::Id, usize)> = BTreeSet::new();
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
        let lexical = sources.iter().any(|s| env.vals.contains_key(s));
        // A source moved by this or an enclosing branch binds after this
        // construct. An export standing in for the whole input would hide it.
        let later = sources
            .iter()
            .any(|(s, _)| moved.contains(s) || env.sentinel.contains(s));
        let merged = env.inputs.contains_key(&(t, i));
        if sentinel(t) {
            // Each source in this construct exports alone. The consumer later
            // passes the sources that fired, as it would get them here. Every
            // branch nested in this one exports them the same way. An earlier
            // export standing in for the whole input would hide them.
            if merged {
                return Err(LowerError::MixedInputSources { node: t, input: i });
            }
            for &(s, o) in &arm_s {
                let var = Var::Output { node: s, output: o };
                src_params.insert((s, o));
                let ret = SlotRet::Param(var);
                slots.insert(
                    var,
                    Slot {
                        ret,
                        missing: Atom::Unfired,
                    },
                );
            }
            for &(s, o) in &cont_s {
                let ret = SlotRet::Val((s, o));
                let var = Var::Output { node: s, output: o };
                slots.insert(
                    var,
                    Slot {
                        ret,
                        missing: Atom::Unfired,
                    },
                );
            }
        } else if !arm_s.is_empty() {
            // Arm-varying sources merge into one scalar param. Mixing them
            // with simultaneously-alive sources is unsupported. Sources
            // visible in neither the region nor the current scope belong to
            // enclosing scopes and merge there.
            if !cont_s.is_empty() || lexical || later || merged {
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
                // A dedicated slot carrying the outlet's single cont source,
                // so a bypassing arm yields the unfired sentinel.
                if cont_s.len() > 1 || lexical || later || merged {
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
                // A sentinel consumer of the same source keeps the sentinel.
                for (s, o) in cont_s {
                    let var = Var::Output { node: s, output: o };
                    slots.entry(var).or_insert(Slot {
                        ret: SlotRet::Val((s, o)),
                        missing: Atom::Unit,
                    });
                }
            }
        }
    }

    // The join body. The cont nodes, with arm-varying inputs reading their
    // params. It ends by yielding the export slots' values.
    let export_vars: Vec<Var> = slots.keys().copied().collect();
    let join_id = cont.first().copied().unwrap_or(b);
    let join = if !cont.is_empty() || !slots.is_empty() {
        let mut join_env = env.clone();
        join_env.sentinel.extend(moved.iter().copied());
        let mut param_vars: Vec<Var> = Vec::new();
        for (&(n, i), &param) in &params {
            join_env.inputs.insert((n, i), Atom::Var(param));
            param_vars.push(param);
        }
        let src_vars = src_params
            .iter()
            .map(|&(s, o)| Var::Output { node: s, output: o });
        param_vars.extend(src_vars);
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

    // Arm bodies. Live arms jump to the join when one exists. They pass each
    // param's value as produced by that arm. Dead arms yield every slot's
    // missing value directly and bypass the join.
    let mut arms = Vec::with_capacity(arm_masks.len());
    for (k, mask) in arm_masks.iter().enumerate() {
        let mut arm_env = env.clone();
        arm_env.sentinel.extend(moved.iter().copied());
        let binds: Vec<Var> = mask
            .iter()
            .enumerate()
            .filter_map(|(o, active)| active.then_some(Var::Output { node: b, output: o }))
            .collect();
        bind_fired(&mut arm_env, &binds);
        let arm_steps = lower_steps(cx, dag, arm_regions[k].clone(), &mut arm_env)?;
        let tail = if live[k] && join.is_some() {
            let mut args = Vec::with_capacity(params.len() + src_params.len());
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
            // A source this arm does not produce passes the unfired sentinel.
            let src_args = src_params
                .iter()
                .map(|src| arm_env.vals.get(src).copied().unwrap_or(Atom::Unfired));
            args.extend(src_args);
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

    // Consume the region. Deferred nodes stay pending and read the exports.
    // So do the moved nodes.
    for n in region.iter() {
        if !deferred.contains(n) {
            pending.remove(n);
        }
    }
    for (&var, slot) in &slots {
        match var {
            Var::Output { node, output } => {
                env.vals.insert((node, output), Atom::Var(var));
                if slot.missing == Atom::Unfired {
                    env.maybe.insert(var);
                }
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

/// The atom an arm passes for the join param merging input `(n, i)`. It is
/// the value of the arm-local source feeding it, or `missing` when this arm
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
    // in-arm sources into an export of its own. Pass that through. A further
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
