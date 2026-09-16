//! The mid-level IR between a gantz graph and Steel code.
//!
//! A graph evaluation lowers to a [`Body`]. That is a sequence of [`Step`]s
//! ending in a [`Tail`]. The IR is Scheme-shaped. Bodies are lexical scopes.
//! Branch reconvergence is a [`Join`] point, a local fn whose parameters
//! carry the reconverging values. A back-edge is a tail [`Tail::Jump`] to a
//! `rec` join. Emission in `emit.rs` is a mechanical walk. All graph
//! reasoning happens in lowering, in `lower.rs`.
//!
//! Invariants are checked by [`validate`]:
//!
//! - Every [`Var`] is bound before use, and never re-bound in scope.
//! - A [`Tail::Jump`] targets a lexically visible join with matching arity.
//!   It may target its own definition only when `rec`.
//! - Every path through a branch arm yields the [`Step::Branch`] export
//!   arity, which is `dst.len()`. It does so directly via [`Tail::Ret`] or
//!   through the join it jumps to.

use crate::node;
use std::collections::{BTreeMap, BTreeSet};

/// Identifies a join point. The smallest node id of the region that
/// reconverges at it, or the loop header for `rec` joins.
pub(crate) type JoinId = node::Id;

/// A reference to a bound value.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum Atom {
    Var(Var),
    /// The empty list `'()`. The "no value" placeholder, for example the
    /// export of a branch arm that does not produce it.
    Unit,
    /// The sentinel marking an outlet value that was never produced. It
    /// distinguishes "did not fire" from "fired with `'()`". It only flows
    /// into outlet-feeding exports and the fired-signature tests that read
    /// them.
    Unfired,
}

/// A variable binding.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum Var {
    /// Output `output` of node `node`. Emitted as `node-{node}-o{output}`.
    Output { node: node::Id, output: usize },
    /// A dedicated binding for input `input` of node `node`. Used as a join
    /// parameter where the value reaching that input varies by branch arm.
    /// Emitted as `node-{node}-i{input}`.
    Input { node: node::Id, input: usize },
    /// The whole-result binding of node `node`. This is a branching node's
    /// `(branch-ix value)` pair. Emitted as `node-{node}`. Bound by a
    /// [`Step::Branch`] with a [`Subject::Call`], or by enclosing glue for a
    /// [`Subject::PreBound`] dispatch.
    Result { node: node::Id },
}

/// An argument to a node fn call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Arg {
    One(Atom),
    /// Multiple sources target the same input. Passed as `(list ...)` in
    /// topological source order.
    List(Vec<Atom>),
}

/// A call to a generated node fn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NodeCall {
    pub node: node::Id,
    /// One element per node input. `None` means unconnected for this
    /// variant. The variant's input mask is derived from which elements are
    /// `Some`.
    pub args: Vec<Option<Arg>>,
    /// The connected-outputs mask. It selects the variant and the result
    /// shape.
    pub outputs: node::Conns,
    pub stateful: bool,
}

/// What a [`Step::Branch`] dispatches on.
#[derive(Debug)]
pub(crate) enum Subject {
    /// Call the branching node's fn, binding its `(branch-ix value)` pair.
    Call(NodeCall),
    /// The pair is already bound as [`Var::Result`] by enclosing glue. For
    /// example, a nested level's evaluation result bridged into this body.
    PreBound { node: node::Id },
}

/// A single statement within a [`Body`].
#[derive(Debug)]
pub(crate) enum Step {
    /// Call a non-branching node fn, binding its connected outputs.
    Node { dst: Vec<Var>, call: NodeCall },
    /// Bind a delay node's stored previous-evaluation value as its output.
    /// Emitted at the top of a level body, before any node runs.
    DelayRead { node: node::Id },
    /// Store the value produced for a delay node's input, to be read on the
    /// next evaluation. Ordered like a node call. Conditional when the
    /// producing path is.
    DelayWrite { node: node::Id, arg: Arg },
    /// Define a join point. Visible to all later steps in this body, the
    /// body's tail, and transitively their nested arms and join bodies.
    Join(Join),
    /// Dispatch on a branching subject's `(branch-ix value)` pair. `dst`
    /// binds the values this statement exports to subsequent steps. Every
    /// path through the arms yields `dst.len()` values.
    Branch {
        subject: Subject,
        dst: Vec<Var>,
        arms: Vec<Arm>,
    },
}

/// One arm of a [`Step::Branch`].
#[derive(Debug)]
pub(crate) struct Arm {
    /// The branch index selecting this arm.
    pub ix: usize,
    /// The output vars bound from the branch value. The arm's active outputs
    /// in ascending output index.
    pub binds: Vec<Var>,
    pub body: Body,
}

/// A join point. A local fn that reconvergent paths tail-call.
#[derive(Debug)]
pub(crate) struct Join {
    pub id: JoinId,
    /// Parameters carry the arm-varying values consumed by the body.
    pub params: Vec<Var>,
    /// Whether the join may jump to itself as a loop header.
    pub rec: bool,
    pub body: Body,
}

/// A lexical block. Steps then a tail.
#[derive(Debug)]
pub(crate) struct Body {
    pub steps: Vec<Step>,
    pub tail: Tail,
}

/// How a body ends.
#[derive(Debug)]
pub(crate) enum Tail {
    /// Yield these values to the enclosing context. That is the branch
    /// statement's exports, or the overall body result.
    Ret(Vec<Atom>),
    /// Tail-call a join point.
    Jump { join: JoinId, args: Vec<Atom> },
}

/// A violation of the IR's scoping or arity invariants.
#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum Invalid {
    #[error("variable {0:?} referenced before binding")]
    UnboundVar(Var),
    #[error("variable {0:?} bound more than once in scope")]
    Rebound(Var),
    #[error("jump to unknown or out-of-scope join {0}")]
    UnknownJoin(JoinId),
    #[error("join {0} defined more than once in scope")]
    DuplicateJoin(JoinId),
    #[error("jump to join {join} with {got} args, expected {expected}")]
    JumpArity {
        join: JoinId,
        expected: usize,
        got: usize,
    },
    #[error("branch on node {node}: arm {arm} yields {got} values, expected {expected}")]
    ExportArity {
        node: node::Id,
        arm: usize,
        expected: usize,
        got: usize,
    },
    #[error("branch on node {0}: duplicate arm index {1}")]
    DuplicateArm(node::Id, usize),
}

/// A join's signature as seen by jumps and yield-arity resolution.
#[derive(Clone, Copy)]
struct JoinSig {
    arity: usize,
    /// The number of values a call to this join evaluates to. `None` while
    /// the join is being validated. A `rec` join's self-jumps yield whatever
    /// the join yields, so they constrain nothing.
    yields: Option<usize>,
}

/// Lexical context threaded through validation. Cloned at scope forks, that
/// is arms and join bodies, so sibling scopes stay independent.
#[derive(Clone, Default)]
struct Scope {
    vars: BTreeSet<Var>,
    joins: BTreeMap<JoinId, JoinSig>,
}

impl Scope {
    fn bind(&mut self, var: Var) -> Result<(), Invalid> {
        if !self.vars.insert(var) {
            return Err(Invalid::Rebound(var));
        }
        Ok(())
    }

    fn check_atom(&self, atom: &Atom) -> Result<(), Invalid> {
        match atom {
            Atom::Unit | Atom::Unfired => Ok(()),
            Atom::Var(v) => self
                .vars
                .contains(v)
                .then_some(())
                .ok_or(Invalid::UnboundVar(*v)),
        }
    }

    fn check_arg(&self, arg: &Arg) -> Result<(), Invalid> {
        match arg {
            Arg::One(a) => self.check_atom(a),
            Arg::List(atoms) => atoms.iter().try_for_each(|a| self.check_atom(a)),
        }
    }

    fn check_call(&self, call: &NodeCall) -> Result<(), Invalid> {
        call.args
            .iter()
            .flatten()
            .try_for_each(|arg| self.check_arg(arg))
    }
}

/// Check the IR invariants for a whole body. `yields` is the number of values
/// it must yield, 0 for an entry fn body. `pre_bound` lists the vars bound by
/// enclosing glue, for example the inlet params of a graph fn.
pub(crate) fn validate(body: &Body, yields: usize, pre_bound: &[Var]) -> Result<(), Invalid> {
    let mut scope = Scope::default();
    for &v in pre_bound {
        scope.bind(v)?;
    }
    let got = validate_body(body, &mut scope)?;
    if got.is_some_and(|got| got != yields) {
        // Reuse ExportArity with a sentinel node id for the top level.
        return Err(Invalid::ExportArity {
            node: node::Id::MAX,
            arm: 0,
            expected: yields,
            got: got.unwrap(),
        });
    }
    Ok(())
}

/// Validate a body within `scope` and return the number of values it yields.
/// `None` when every path ends in a self-jump to an enclosing rec join. The
/// yield is then the join's own and constrains nothing.
fn validate_body(body: &Body, scope: &mut Scope) -> Result<Option<usize>, Invalid> {
    for step in &body.steps {
        match step {
            Step::Node { dst, call } => {
                scope.check_call(call)?;
                for &v in dst {
                    scope.bind(v)?;
                }
            }
            Step::DelayRead { node } => {
                scope.bind(Var::Output {
                    node: *node,
                    output: 0,
                })?;
            }
            Step::DelayWrite { node: _, arg } => {
                scope.check_arg(arg)?;
            }
            Step::Join(join) => {
                if scope.joins.contains_key(&join.id) {
                    return Err(Invalid::DuplicateJoin(join.id));
                }
                let mut inner = scope.clone();
                for &p in &join.params {
                    inner.bind(p)?;
                }
                // A rec join is visible within its own body with a deferred
                // `None` yield. Self-jump paths yield whatever the join
                // yields, so they constrain nothing.
                if join.rec {
                    inner.joins.insert(
                        join.id,
                        JoinSig {
                            arity: join.params.len(),
                            yields: None,
                        },
                    );
                }
                let yields = validate_body(&join.body, &mut inner)?;
                scope.joins.insert(
                    join.id,
                    JoinSig {
                        arity: join.params.len(),
                        yields,
                    },
                );
            }
            Step::Branch { subject, dst, arms } => {
                let node = match subject {
                    Subject::Call(call) => {
                        scope.check_call(call)?;
                        call.node
                    }
                    Subject::PreBound { node } => {
                        let pair = Var::Result { node: *node };
                        if !scope.vars.contains(&pair) {
                            return Err(Invalid::UnboundVar(pair));
                        }
                        *node
                    }
                };
                let mut seen = BTreeSet::new();
                for arm in arms {
                    if !seen.insert(arm.ix) {
                        return Err(Invalid::DuplicateArm(node, arm.ix));
                    }
                    let mut inner = scope.clone();
                    for &b in &arm.binds {
                        inner.bind(b)?;
                    }
                    let yields = validate_body(&arm.body, &mut inner)?;
                    if yields.is_some_and(|got| got != dst.len()) {
                        return Err(Invalid::ExportArity {
                            node,
                            arm: arm.ix,
                            expected: dst.len(),
                            got: yields.unwrap(),
                        });
                    }
                }
                for &v in dst {
                    scope.bind(v)?;
                }
            }
        }
    }

    match &body.tail {
        Tail::Ret(atoms) => {
            for a in atoms {
                scope.check_atom(a)?;
            }
            Ok(Some(atoms.len()))
        }
        Tail::Jump { join, args } => {
            for a in args {
                scope.check_atom(a)?;
            }
            let sig = scope.joins.get(join).ok_or(Invalid::UnknownJoin(*join))?;
            if sig.arity != args.len() {
                return Err(Invalid::JumpArity {
                    join: *join,
                    expected: sig.arity,
                    got: args.len(),
                });
            }
            Ok(sig.yields)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(node: node::Id, output: usize) -> Var {
        Var::Output { node, output }
    }

    fn call(node: node::Id, args: Vec<Option<Arg>>, n_outputs: usize) -> NodeCall {
        NodeCall {
            node,
            args,
            outputs: node::Conns::connected(n_outputs).unwrap(),
            stateful: false,
        }
    }

    /// `(define node-0-o0 (node-fn-0)) (define node-1-o0 (node-fn-1 node-0-o0))`
    #[test]
    fn linear_chain_validates() {
        let body = Body {
            steps: vec![
                Step::Node {
                    dst: vec![out(0, 0)],
                    call: call(0, vec![], 1),
                },
                Step::Node {
                    dst: vec![out(1, 0)],
                    call: call(1, vec![Some(Arg::One(Atom::Var(out(0, 0))))], 1),
                },
            ],
            tail: Tail::Ret(vec![]),
        };
        validate(&body, 0, &[]).unwrap();
    }

    #[test]
    fn unbound_var_rejected() {
        let body = Body {
            steps: vec![Step::Node {
                dst: vec![out(1, 0)],
                call: call(1, vec![Some(Arg::One(Atom::Var(out(0, 0))))], 1),
            }],
            tail: Tail::Ret(vec![]),
        };
        assert_eq!(validate(&body, 0, &[]), Err(Invalid::UnboundVar(out(0, 0))));
    }

    #[test]
    fn rebound_var_rejected() {
        let body = Body {
            steps: vec![
                Step::Node {
                    dst: vec![out(0, 0)],
                    call: call(0, vec![], 1),
                },
                Step::Node {
                    dst: vec![out(0, 0)],
                    call: call(0, vec![], 1),
                },
            ],
            tail: Tail::Ret(vec![]),
        };
        assert_eq!(validate(&body, 0, &[]), Err(Invalid::Rebound(out(0, 0))));
    }

    /// A branch whose arms jump to a join. The join body consumes the param.
    #[test]
    fn branch_with_join_validates() {
        let param = Var::Input { node: 3, input: 0 };
        let join = Join {
            id: 3,
            params: vec![param],
            rec: false,
            body: Body {
                steps: vec![Step::Node {
                    dst: vec![out(3, 0)],
                    call: call(3, vec![Some(Arg::One(Atom::Var(param)))], 1),
                }],
                tail: Tail::Ret(vec![]),
            },
        };
        let arm = |ix: usize| Arm {
            ix,
            binds: vec![out(1, ix)],
            body: Body {
                steps: vec![],
                tail: Tail::Jump {
                    join: 3,
                    args: vec![Atom::Var(out(1, ix))],
                },
            },
        };
        let body = Body {
            steps: vec![
                Step::Join(join),
                Step::Branch {
                    subject: Subject::Call(call(1, vec![], 2)),
                    dst: vec![],
                    arms: vec![arm(0), arm(1)],
                },
            ],
            tail: Tail::Ret(vec![]),
        };
        validate(&body, 0, &[]).unwrap();
    }

    /// Arm yield arity must match the branch's export count, whether the arm
    /// rets directly or through a join.
    #[test]
    fn export_arity_mismatch_rejected() {
        let body = Body {
            steps: vec![Step::Branch {
                subject: Subject::Call(call(1, vec![], 2)),
                dst: vec![out(9, 0)],
                arms: vec![Arm {
                    ix: 0,
                    binds: vec![out(1, 0)],
                    body: Body {
                        steps: vec![],
                        tail: Tail::Ret(vec![]),
                    },
                }],
            }],
            tail: Tail::Ret(vec![]),
        };
        assert_eq!(
            validate(&body, 0, &[]),
            Err(Invalid::ExportArity {
                node: 1,
                arm: 0,
                expected: 1,
                got: 0,
            })
        );
    }

    /// Sibling arm scopes are independent. Both arms may bind the same vars.
    #[test]
    fn sibling_arms_bind_same_vars() {
        let arm = |ix: usize| Arm {
            ix,
            binds: vec![out(1, 0)],
            body: Body {
                steps: vec![],
                tail: Tail::Ret(vec![Atom::Var(out(1, 0))]),
            },
        };
        let body = Body {
            steps: vec![Step::Branch {
                subject: Subject::Call(call(1, vec![], 1)),
                dst: vec![out(1, 0)],
                arms: vec![arm(0), arm(1)],
            }],
            tail: Tail::Ret(vec![]),
        };
        validate(&body, 0, &[]).unwrap();
    }

    #[test]
    fn jump_to_undefined_join_rejected() {
        let body = Body {
            steps: vec![],
            tail: Tail::Jump {
                join: 7,
                args: vec![],
            },
        };
        assert_eq!(validate(&body, 0, &[]), Err(Invalid::UnknownJoin(7)));
    }

    /// A rec join may jump to itself. A non-rec join's body cannot see itself.
    #[test]
    fn rec_join_self_jump() {
        let rec_join = |rec: bool| Body {
            steps: vec![Step::Join(Join {
                id: 5,
                params: vec![],
                rec,
                body: Body {
                    steps: vec![],
                    tail: Tail::Jump {
                        join: 5,
                        args: vec![],
                    },
                },
            })],
            tail: Tail::Ret(vec![]),
        };
        validate(&rec_join(true), 0, &[]).unwrap();
        assert_eq!(
            validate(&rec_join(false), 0, &[]),
            Err(Invalid::UnknownJoin(5))
        );
    }
}
