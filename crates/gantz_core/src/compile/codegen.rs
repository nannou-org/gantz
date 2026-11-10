//! Generation of the per-variant node fns.
//!
//! In gantz, a function is generated for every unique configuration of every
//! node. That is, for each unique set of connected inputs and outputs of a
//! node, a function is generated.
//!
//! These configurations are collected by traversing from each of the push/pull
//! evaluation entrypoints. With
//! [`Config::emit_all_node_fns`][crate::compile::Config], every node's
//! all-connected configuration is included as well.

use crate::{
    Edge,
    compile::{
        NodeConf, NodeConns, RoseTree, emit,
        error::{NestedGraphNotFound, NodeExprError, NodeFnError, NodeFnErrors},
        names::{STATE, node_fn_name},
    },
    node::{self, Node},
    visit::{self, Visitor},
};
use petgraph::visit::{Data, IntoEdgesDirected, IntoNodeReferences, NodeIndexable, Visitable};
use std::collections::{BTreeMap, BTreeSet};
use steel::parser::ast::ExprKind;

/// How a node fn variant uses the state of its node.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StateUse {
    /// The fn takes the state as its last argument and returns its output.
    Read,
    /// The fn takes the state as its last argument and returns
    /// `(list output state)`, with the state it leaves behind.
    Write,
}

/// The node fns generated for a tree of nested gantz graphs.
#[derive(Default)]
pub(crate) struct NodeFns {
    /// Each fn with the depth of the level of its node.
    pub fns: Vec<(usize, ExprKind)>,
    /// The state use of each stateful variant, by level path.
    pub state: StateUses,
}

/// A visitor used to collect all node functions from a tree of nested gantz
/// graphs.
struct NodeFnsVisitor<'a> {
    tree: &'a RoseTree<NodeConfs>,
    fns: NodeFns,
    errors: Vec<NodeFnError>,
}

/// The set of all node input/output configurations for a single graph.
///
/// These are used to determine which set of functions to generate for each
/// node.
type NodeConfs = BTreeSet<NodeConf>;

/// The state use of every stateful node fn variant, by level path.
pub(crate) type StateUses = BTreeMap<Vec<node::Id>, BTreeMap<NodeConf, StateUse>>;

impl Visitor for NodeFnsVisitor<'_> {
    // `visit_post` generates nested node fns before their parents.
    fn visit_post(&mut self, ctx: visit::Ctx<'_, '_>, node: &dyn Node) {
        // Inlet and outlet nodes get no node fn. Their values resolve as
        // bindings.
        let meta_ctx = node::MetaCtx::new(ctx.get_node());
        if matches!(node.role(meta_ctx), node::Role::Inlet | node::Role::Outlet) {
            return;
        }

        use std::ops::Bound::{Excluded, Included};
        let node_path = ctx.path();
        let plan_path = &node_path[..node_path.len() - 1];
        let tree = match self.tree.tree(plan_path) {
            Some(t) => t,
            None => {
                self.errors
                    .push(NestedGraphNotFound(plan_path.to_vec()).into());
                return;
            }
        };
        let id = ctx.id();
        let empty_conns = NodeConns {
            inputs: node::Conns::empty(),
            outputs: node::Conns::empty(),
        };
        let start = NodeConf {
            id,
            conns: empty_conns.clone(),
        };
        let end_id = id.checked_add(1).expect("node id out of range");
        let end = NodeConf {
            id: end_id,
            conns: empty_conns,
        };
        let range = (Included(start), Excluded(end));
        let input_confs = tree.elem.range(range);
        for conf in input_confs {
            let connected = connected_inputs(conf.conns.inputs, ctx.inputs());
            match node_fn(ctx.get_node(), node, node_path, &conf.conns, &connected) {
                Ok((fn_expr, state)) => {
                    self.fns.fns.push((plan_path.len(), fn_expr));
                    if let Some(state) = state {
                        let level = self.fns.state.entry(plan_path.to_vec()).or_default();
                        level.insert(*conf, state);
                    }
                }
                Err(error) => self.errors.push(
                    NodeExprError {
                        path: node_path.to_vec(),
                        error,
                    }
                    .into(),
                ),
            }
        }
    }
}

/// The inputs of a node that an edge targets, whether or not they fire.
///
/// `active` gives the length. An edge to an input index out of range never
/// reaches the node.
fn connected_inputs(active: node::Conns, edges: &[(node::Id, Edge)]) -> node::Conns {
    let mut connected = active;
    for (_, edge) in edges {
        connected.set(edge.input.0 as usize, true).ok();
    }
    connected
}

/// Whether `expr` may assign the `state` binding of its node fn.
///
/// True when a `(set! state ...)` form appears anywhere in the expr. A macro
/// definition also counts, as its expansion is unknown here.
fn writes_state(expr: &ExprKind) -> bool {
    fn any(exprs: &[ExprKind]) -> bool {
        exprs.iter().any(writes_state)
    }
    match expr {
        ExprKind::Atom(_) | ExprKind::Require(_) => false,
        ExprKind::Macro(_) | ExprKind::SyntaxRules(_) => true,
        ExprKind::Set(set) => {
            let state = set.variable.atom_identifier();
            state.is_some_and(|ident| ident.resolve() == STATE) || writes_state(&set.expr)
        }
        ExprKind::If(f) => {
            writes_state(&f.test_expr) || writes_state(&f.then_expr) || writes_state(&f.else_expr)
        }
        ExprKind::Let(l) => {
            l.bindings.iter().any(|(_, value)| writes_state(value)) || writes_state(&l.body_expr)
        }
        ExprKind::Define(d) => writes_state(&d.body),
        ExprKind::LambdaFunction(f) => writes_state(&f.body),
        ExprKind::Begin(b) => any(&b.exprs),
        ExprKind::Return(r) => writes_state(&r.expr),
        ExprKind::Quote(q) => writes_state(&q.expr),
        // A `set!` under a quote stays a plain list. It counts as a write, as
        // the quote may be code that a macro splices.
        ExprKind::List(l) => {
            let set = l
                .args
                .first()
                .is_some_and(|head| head.to_string() == "set!");
            let state = l.args.get(1).and_then(ExprKind::atom_identifier);
            (set && state.is_some_and(|ident| ident.resolve() == STATE)) || any(&l.args)
        }
        ExprKind::Vector(v) => any(&v.args),
    }
}

/// Generate a function for a single node with the given set of active inputs.
///
/// Also returns how the fn uses the state of the node, or `None` for a node
/// without state.
pub(crate) fn node_fn<'a>(
    get_node: node::GetNode<'a>,
    node: &dyn Node,
    node_path: &[node::Id],
    conns: &NodeConns,
    connected: &node::Conns,
) -> Result<(ExprKind, Option<StateUse>), node::ExprError> {
    let input_exprs: Vec<Option<String>> = conns
        .inputs
        .iter()
        .enumerate()
        .map(|(i, b)| b.then(|| format!("input{i}")))
        .collect();
    let mut params: Vec<String> = input_exprs.iter().flatten().cloned().collect();

    let ctx = node::ExprCtx::new(get_node, node_path, &input_exprs, connected, &conns.outputs);
    let node_expr = node.expr(ctx)?;

    let fn_name = node_fn_name(node_path, &conns.inputs, &conns.outputs);
    let meta_ctx = node::MetaCtx::new(get_node);
    let state = node
        .stateful(meta_ctx)
        .then(|| match writes_state(&node_expr) {
            true => StateUse::Write,
            false => StateUse::Read,
        });
    if state.is_none()
        && let Some(f) = emit::eta_target_expr(&params, &node_expr)
    {
        return Ok((node::parse_expr(&format!("(define {fn_name} {f})"))?, None));
    }
    let fn_body = match state {
        None => format!("{node_expr}"),
        Some(StateUse::Read) => {
            params.push(STATE.to_string());
            format!("{node_expr}")
        }
        Some(StateUse::Write) => {
            params.push(STATE.to_string());
            format!("(let ((output {node_expr})) (list output {STATE}))")
        }
    };
    let params = params.join(" ");
    let fn_def = format!("(define ({fn_name} {params}) {fn_body})");
    Ok((node::parse_expr(&fn_def)?, state))
}

/// Given a gantz graph and a rose tree with the associated node configs,
/// produce a function for every node configuration in the graph.
pub(crate) fn node_fns<'a, G>(
    get_node: node::GetNode<'a>,
    g: G,
    node_confs_tree: &RoseTree<NodeConfs>,
) -> Result<NodeFns, NodeFnErrors>
where
    G: Data<EdgeWeight = Edge> + IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable,
    G::NodeWeight: Node,
{
    let mut visitor = NodeFnsVisitor {
        tree: node_confs_tree,
        fns: NodeFns::default(),
        errors: vec![],
    };
    crate::graph::visit(get_node, g, &[], &mut visitor);
    if !visitor.errors.is_empty() {
        return Err(NodeFnErrors(visitor.errors));
    }
    Ok(visitor.fns)
}

#[cfg(test)]
mod tests {
    use super::writes_state;
    use crate::node::parse_expr;

    #[test]
    fn set_of_state_is_a_write() {
        let writes = [
            "(set! state 1)",
            "(begin (set! state (+ state 1)) state)",
            "(begin (if (number? x) (set! state x) void) state)",
            "(let ((r (f state))) (set! state (car r)) (cdr r))",
            "(lambda (x) (set! state x))",
            "(f (list 1 (set! state 2)))",
            "(set! other (set! state 1))",
        ];
        for src in writes {
            assert!(writes_state(&parse_expr(src).unwrap()), "{src}");
        }
    }

    #[test]
    fn read_of_state_is_no_write() {
        let reads = [
            "state",
            "(+ state 1)",
            "(begin (set! other state) state)",
            "(let ((estate 1)) (set! estate 2))",
            "(if (number? state) state 0)",
            "(lambda (x) (+ x state))",
        ];
        for src in reads {
            assert!(!writes_state(&parse_expr(src).unwrap()), "{src}");
        }
    }

    #[test]
    fn unknown_expansion_is_a_write() {
        let src = "(begin (define-syntax put! (syntax-rules () [(_ v) (set! state v)])) (put! 1))";
        assert!(writes_state(&parse_expr(src).unwrap()));
        assert!(writes_state(&parse_expr("'(set! state 1)").unwrap()));
    }
}
