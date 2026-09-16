//! The outlet-activation analysis. Which sets of a level's outlets can fire
//! together, across every combination of branch outcomes.
//!
//! Computed by abstract interpretation of the lowered IR itself. The only
//! question is whether each outlet's value is real or the unfired sentinel.
//! Bodies are walked with every binding abstracted to fired or unfired. The
//! walk forks at each branch dispatch. Nested dispatches fork only when
//! their arm is taken. The walk runs over exactly the code that is emitted,
//! so the patterns cannot drift from runtime behaviour.
//!
//! Backs `Graph::branches` via [`level_branch_patterns`]. These are the
//! masks a nested graph reports to its parent. Also backs the
//! push-through-outlet propagation in [`super::module`] via
//! [`outlet_patterns`] over each entrypoint's level body.

use crate::{
    compile::{
        Meta,
        error::{LowerError, TooManyConns},
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

/// The abstract environment. Bindings not present default to fired. Inlet
/// params, pre-bound values and node outputs are always real values.
type Env = BTreeMap<Var, Fire>;

/// The join definitions lexically in scope.
type Joins<'a> = BTreeMap<JoinId, &'a Join>;

/// The all-active external branch masks for one level. The distinct
/// outlet-activation patterns over the level's outputs in ascending outlet
/// id order. Empty when fewer than two are reachable, since there is then no
/// external branching. Backs `Graph::branches` and the graph fn's result
/// selector.
pub(crate) fn level_branch_patterns(meta: &Meta) -> Result<Vec<node::Conns>, LowerError> {
    // No inner branching means no external branching.
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
    let patterns = outlet_patterns(&out.body, &out.outlets).map_err(|error| LowerError::Conns {
        node: None,
        error: error.into(),
    })?;
    Ok(patterns)
}

/// The distinct outlet-activation masks a lowered level can produce, in
/// ascending outlet order. An empty `Vec` when fewer than two are possible,
/// since there is then no external branching.
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

/// Every `(yield, final env)` outcome of evaluating `steps` then `tail`.
/// One per combination of branch arms taken along the way.
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
        // Fork. Each arm's outcomes bind the exports, then the remaining
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

/// The outcomes of a body's tail. The yielded values. For a jump, the
/// outcomes of the join body with its params bound from the args.
fn finish<'a>(tail: &'a Tail, env: Env, joins: &Joins<'a>) -> Vec<(Vec<Fire>, Env)> {
    match tail {
        Tail::Ret(atoms) => {
            let yields = atoms.iter().map(|a| fire_of(&env, a)).collect();
            vec![(yields, env)]
        }
        Tail::Jump { join, args } => {
            let join = joins[join];
            // The lowering does not produce `rec` joins yet. Their self-jumps
            // would need fixpoint handling here rather than unbounded
            // recursion.
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
