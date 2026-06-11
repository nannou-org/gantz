//! The outlet-activation analysis: which sets of a level's outlets can fire
//! together, across every combination of branch outcomes.
//!
//! Computed by abstract interpretation of the *lowered IR itself*: the only
//! question is whether each outlet's value is real or the unfired sentinel,
//! so bodies are walked with every binding abstracted to fired/unfired,
//! forking at each branch dispatch (nested dispatches fork only when their
//! arm is actually taken). Because the walk runs over exactly the code that
//! is emitted, the patterns cannot drift from runtime behaviour.
//!
//! Backs `Graph::branches` (the masks a nested graph reports to its parent,
//! via [`level_branch_patterns`]) and the push-through-outlet propagation in
//! [`super::module`] (via [`outlet_patterns`] over each entrypoint's level
//! body).

use crate::{
    compile::{
        Meta,
        error::{LowerError, NodeConnsError, TooManyConns},
        ir::{Atom, Body, Join, JoinId, Step, Tail, Var},
        lower::{self, LevelSources, OutletVal},
    },
    node,
};
use std::collections::{BTreeMap, BTreeSet};

/// Whether a binding holds a real value or the unfired sentinel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Fire {
    Fired,
    Unfired,
}

/// The abstract environment: bindings not present default to fired (inlet
/// params, pre-bound values and node outputs are always real values).
type Env = BTreeMap<Var, Fire>;

/// The join definitions lexically in scope.
type Joins<'a> = BTreeMap<JoinId, &'a Join>;

/// The all-active external branch masks for one level: the distinct
/// outlet-activation patterns over the level's outputs (ascending outlet id
/// order), or empty when fewer than two are reachable (no external
/// branching). Backs `Graph::branches` and the graph fn's result selector.
pub(crate) fn level_branch_patterns(meta: &Meta) -> Result<Vec<node::Conns>, LowerError> {
    // No inner branching => no external branching.
    if meta.branches.is_empty() {
        return Ok(vec![]);
    }
    let all: BTreeSet<node::Id> = meta.inlets.iter().copied().collect();
    let cx = lower::Cx {
        meta,
        extra_branches: BTreeMap::new(),
        prebound: BTreeSet::new(),
    };
    let out = lower::level_body(&cx, &LevelSources::Inlets(all))?;
    let patterns = outlet_patterns(&out.body, &out.outlets).map_err(NodeConnsError::from)?;
    Ok(patterns)
}

/// The distinct outlet-activation masks (ascending outlet order) a lowered
/// level can produce, or an empty `Vec` when fewer than two are possible
/// (no external branching).
pub(crate) fn outlet_patterns(
    body: &Body,
    outlets: &[OutletVal],
) -> Result<Vec<node::Conns>, TooManyConns> {
    let n = outlets.len();
    let mut masks: BTreeSet<node::Conns> = BTreeSet::new();
    for (_, env) in walk(&body.steps, &body.tail, Env::new(), &Joins::new()) {
        let mut conns = node::Conns::unconnected(n).map_err(|_| TooManyConns(n))?;
        for (i, o) in outlets.iter().enumerate() {
            let fired = match o.atom {
                None => false,
                Some(ref atom) => fire_of(&env, atom) == Fire::Fired,
            };
            if fired {
                conns.set(i, true).map_err(|_| TooManyConns(n))?;
            }
        }
        masks.insert(conns);
    }
    if masks.len() < 2 {
        return Ok(vec![]);
    }
    Ok(masks.into_iter().collect())
}

/// The abstract value of `atom` under `env`.
fn fire_of(env: &Env, atom: &Atom) -> Fire {
    match atom {
        Atom::Unfired => Fire::Unfired,
        Atom::Unit => Fire::Fired,
        Atom::Var(v) => env.get(v).copied().unwrap_or(Fire::Fired),
    }
}

/// Every `(yield, final env)` outcome of evaluating `steps` then `tail` -
/// one per combination of branch arms taken along the way.
fn walk<'a>(
    steps: &'a [Step],
    tail: &'a Tail,
    env: Env,
    joins: &Joins<'a>,
) -> Vec<(Vec<Fire>, Env)> {
    let Some((step, rest)) = steps.split_first() else {
        return finish(tail, env, joins);
    };
    match step {
        // A node fn call always yields real values.
        Step::Node { dst, .. } => {
            let mut env = env;
            for &v in dst {
                env.insert(v, Fire::Fired);
            }
            walk(rest, tail, env, joins)
        }
        Step::DelayRead { node } => {
            let mut env = env;
            let var = Var::Output {
                node: *node,
                output: 0,
            };
            env.insert(var, Fire::Fired);
            walk(rest, tail, env, joins)
        }
        Step::DelayWrite { .. } => walk(rest, tail, env, joins),
        Step::Join(join) => {
            let mut joins = joins.clone();
            joins.insert(join.id, join);
            walk(rest, tail, env, &joins)
        }
        // Fork: each arm's outcomes bind the exports, then the remaining
        // steps continue per outcome.
        Step::Branch { dst, arms, .. } => {
            let mut outcomes = Vec::new();
            for arm in arms {
                let mut arm_env = env.clone();
                for &b in &arm.binds {
                    arm_env.insert(b, Fire::Fired);
                }
                for (yields, arm_env) in walk(&arm.body.steps, &arm.body.tail, arm_env, joins) {
                    let mut env = arm_env;
                    for (&v, f) in dst.iter().zip(yields) {
                        env.insert(v, f);
                    }
                    outcomes.extend(walk(rest, tail, env, joins));
                }
            }
            outcomes
        }
    }
}

/// The outcomes of a body's tail: the yielded values, or - for a jump - the
/// outcomes of the join body with its params bound from the args.
fn finish<'a>(tail: &'a Tail, env: Env, joins: &Joins<'a>) -> Vec<(Vec<Fire>, Env)> {
    match tail {
        Tail::Ret(atoms) => {
            let yields = atoms.iter().map(|a| fire_of(&env, a)).collect();
            vec![(yields, env)]
        }
        Tail::Jump { join, args } => {
            let join = joins[join];
            // The lowering does not produce `rec` joins yet (reserved for
            // iterate-until-branch loops); their self-jumps would need
            // fixpoint handling here rather than unbounded recursion.
            assert!(!join.rec, "outlet analysis cannot walk rec joins yet");
            let mut env = env;
            for (&param, arg) in join.params.iter().zip(args) {
                let fire = fire_of(&env, arg);
                env.insert(param, fire);
            }
            walk(&join.body.steps, &join.body.tail, env, joins)
        }
    }
}
