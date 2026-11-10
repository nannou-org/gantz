//! The outlet-activation analysis. Which sets of a level's outlets can fire
//! together, across every combination of branch outcomes.
//!
//! Computed by abstract interpretation of the lowered IR itself. The only
//! question is whether each outlet's value is real or the unfired sentinel.
//! Bodies are walked with every binding abstracted to fired or unfired. The
//! walk forks at each branch dispatch and keeps one outcome per distinct set
//! of unfired bindings. Nested dispatches fork only when their arm is taken.
//! The walk runs over exactly the code that is emitted, so the patterns
//! cannot drift from runtime behaviour.
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
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Fire {
    Fired,
    Unfired,
}

/// The state shared by one walk of a level body.
#[derive(Default)]
struct Walk {
    /// The vars that an outlet or a tail reads. Only these can change which
    /// outlets fire, so the envs track no other var.
    read: BTreeSet<Var>,
    /// The yields of each join body already walked, by the env it started
    /// from. The key is the address of the join, as two joins in sibling
    /// scopes can have the same id.
    joined: BTreeMap<(*const Join, Env), BTreeSet<Vec<Fire>>>,
}

/// The abstract environment. The bindings that hold the unfired sentinel.
/// Every other binding holds a real value. Inlet params, pre-bound values and
/// node outputs always do.
type Env = BTreeSet<Var>;

/// The join definitions lexically in scope. Each comes with the unfired
/// bindings that its body can read, those in an env at its definition.
type Joins<'a> = BTreeMap<JoinId, (&'a Join, Env)>;

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
    let mut walk = Walk::default();
    let outlet_vars = outlets.iter().filter_map(|o| match o.atom {
        Some(Atom::Var(var)) => Some(var),
        _ => None,
    });
    walk.read.extend(outlet_vars);
    read_vars(body, &mut walk.read);

    let mut masks: BTreeSet<node::Conns> = BTreeSet::new();
    for env in steps_envs(&body.steps, Env::new(), &mut Joins::new(), &mut walk) {
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

/// Add the vars that a tail of `body` reads, at any depth.
fn read_vars(body: &Body, read: &mut BTreeSet<Var>) {
    for step in &body.steps {
        match step {
            Step::Node { .. } | Step::DelayRead { .. } | Step::DelayWrite { .. } => {}
            Step::Join(join) => read_vars(&join.body, read),
            Step::Branch { arms, .. } => arms.iter().for_each(|arm| read_vars(&arm.body, read)),
        }
    }
    let (Tail::Ret(atoms) | Tail::Jump { args: atoms, .. }) = &body.tail;
    read.extend(atoms.iter().filter_map(|atom| match atom {
        Atom::Var(var) => Some(*var),
        Atom::Unit | Atom::Unfired => None,
    }));
}

/// The abstract value of `atom` under `env`.
fn fire_of(env: &Env, atom: &Atom) -> Fire {
    match atom {
        Atom::Unfired => Fire::Unfired,
        Atom::Unit => Fire::Fired,
        Atom::Var(v) if env.contains(v) => Fire::Unfired,
        Atom::Var(_) => Fire::Fired,
    }
}

/// The distinct envs that `steps` can end in when they start from `env`.
/// There is one per set of unfired exports that the branch arms taken along
/// the way can leave. `joins` gains the joins that the steps define.
fn steps_envs<'a>(
    steps: &'a [Step],
    env: Env,
    joins: &mut Joins<'a>,
    walk: &mut Walk,
) -> BTreeSet<Env> {
    let mut envs = BTreeSet::from([env]);
    for step in steps {
        match step {
            // A node fn call and a delay read always bind real values.
            Step::Node { .. } | Step::DelayRead { .. } | Step::DelayWrite { .. } => {}
            Step::Join(join) => {
                let scope = envs.iter().flatten().copied().collect();
                joins.insert(join.id, (join, scope));
            }
            // Each arm yields the exports. The vars an arm binds are out of
            // scope after it, so each outcome is the env from before the
            // branch with the unfired exports added.
            Step::Branch { dst, arms, .. } => {
                let mut next = BTreeSet::new();
                for env in &envs {
                    for arm in arms {
                        for yields in body_yields(&arm.body, env.clone(), joins, walk) {
                            let mut env = env.clone();
                            let unfired = dst.iter().zip(yields).filter_map(|(var, fire)| {
                                (fire == Fire::Unfired && walk.read.contains(var)).then_some(*var)
                            });
                            env.extend(unfired);
                            next.insert(env);
                        }
                    }
                }
                envs = next;
            }
        }
    }
    envs
}

/// The distinct yields of `body` when it starts from `env`. For a jump, they
/// are the yields of the join body with its params bound from the args.
fn body_yields<'a>(
    body: &'a Body,
    env: Env,
    joins: &Joins<'a>,
    walk: &mut Walk,
) -> BTreeSet<Vec<Fire>> {
    let mut joins = joins.clone();
    let mut yields = BTreeSet::new();
    for env in steps_envs(&body.steps, env, &mut joins, walk) {
        let (join, args) = match &body.tail {
            Tail::Ret(atoms) => {
                yields.insert(atoms.iter().map(|a| fire_of(&env, a)).collect());
                continue;
            }
            Tail::Jump { join, args } => (join, args),
        };
        let (join, scope) = &joins[join];
        // The lowering does not produce `rec` joins yet. Their self-jumps
        // would need fixpoint handling here rather than unbounded recursion.
        assert!(!join.rec, "outlet analysis cannot walk rec joins yet");
        // The join body reads its params and the vars bound before the join.
        let mut join_env: Env = env.intersection(scope).copied().collect();
        let unfired = join
            .params
            .iter()
            .zip(args)
            .filter_map(|(param, arg)| (fire_of(&env, arg) == Fire::Unfired).then_some(*param));
        join_env.extend(unfired);
        let key = (std::ptr::from_ref::<Join>(join), join_env);
        if !walk.joined.contains_key(&key) {
            let found = body_yields(&join.body, key.1.clone(), &joins, walk);
            walk.joined.insert(key.clone(), found);
        }
        yields.extend(walk.joined[&key].iter().cloned());
    }
    yields
}
