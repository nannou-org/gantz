//! Items related to the generation of node functions.
//!
//! In gantz, a function is generated for every unique configuration of every
//! node. That is, for each unique set of connected inputs and outputs of a
//! node, a function is generated.
//!
//! These configurations are collected by traversing from each of the push/pull
//! evaluation entrypoints.

use super::{EvalPlan, EvalStep, RoseTree};
use crate::{
    Edge,
    node::{self, Node},
    visit::{self, Visitor},
};
use petgraph::visit::{Data, IntoEdgesDirected, IntoNodeReferences, NodeIndexable, Visitable};
use std::collections::BTreeSet;
use steel::{parser::ast::ExprKind, steel_vm::engine::Engine};

/// The set of all node input/output configurations for a single graph.
///
/// These are used to determine which set of functions to generate for each
/// node.
type NodeConfs = BTreeSet<(node::Id, NodeConf)>;

/// The connectedness of a node for a particular evaluation step.
#[derive(Clone, Eq, PartialEq, PartialOrd, Ord)]
pub(super) struct NodeConf {
    inputs: Vec<bool>,
    outputs: Vec<bool>,
}

/// A visitor used to collect all node functions from a tree of nested gantz
/// graphs.
struct NodeFns<'a> {
    tree: &'a RoseTree<NodeConfs>,
    fns: Vec<ExprKind>,
}

impl<'a> NodeFns<'a> {
    /// Initialise the `NodeFns` visitor.
    fn new(tree: &'a RoseTree<NodeConfs>) -> Self {
        let fns = vec![];
        Self { tree, fns }
    }
}

impl<'pl> Visitor for NodeFns<'pl> {
    // We use `visit_post` so that the nested are generated before parents.
    fn visit_post(&mut self, ctx: visit::Ctx, node: &dyn Node) {
        use std::ops::Bound::{Excluded, Included};
        let node_path = ctx.path();
        let plan_path = &node_path[..node_path.len() - 1];
        let tree = self.tree.tree(&plan_path).unwrap();
        let id = ctx.id();
        let empty_conf = NodeConf {
            inputs: vec![],
            outputs: vec![],
        };
        let start = (id, empty_conf.clone());
        let end = (id.checked_add(1).expect("node id out of range"), empty_conf);
        let range = (Included(start), Excluded(end));
        let input_confs = tree.elem.range(range);
        for (_id, conf) in input_confs {
            self.fns.push(node_fn(node, node_path, &conf));
        }
    }
}

/// Collect all unique node configurations for all unique evaluation paths that
/// exist in the graph.
fn node_confs<'a, I>(eval_stepss: I) -> NodeConfs
where
    I: IntoIterator<Item = &'a [EvalStep]>,
{
    eval_stepss
        .into_iter()
        .flat_map(|steps| {
            steps.iter().map(|step| {
                let inputs = step.inputs.iter().map(|input| input.is_some()).collect();
                let outputs = step.outputs.clone();
                let conf = NodeConf { inputs, outputs };
                (step.node, conf)
            })
        })
        .collect()
}

/// Construct a rose tree of node configs from a tree of eval plans.
pub(super) fn node_confs_tree(eval_tree: &RoseTree<EvalPlan>) -> RoseTree<NodeConfs> {
    eval_tree.map_ref(&mut |eval| {
        let all_steps = eval
            .pull_steps
            .values()
            .chain(eval.push_steps.values())
            .chain(Some(&eval.nested_steps))
            .map(|v| &v[..]);
        node_confs(all_steps)
    })
}

/// Generate a function name for a node based on its path in the graph.
pub(crate) fn node_fn_name(node_path: &[node::Id], inputs: &[bool], outputs: &[bool]) -> String {
    let path_string = super::path_string(node_path);
    let bin_string =
        |bin: &[bool]| -> String { bin.iter().map(|&b| if b { "1" } else { "0" }).collect() };
    let inputs_prefix = if inputs.is_empty() { "" } else { "_i" };
    let outputs_prefix = if outputs.is_empty() { "" } else { "_o" };
    let inputs_string = bin_string(inputs);
    let outputs_string = bin_string(outputs);
    format!("node_fn_{path_string}{inputs_prefix}{inputs_string}{outputs_prefix}{outputs_string}")
}

/// Generate a function for a single node with the given set of connected inputs.
pub(crate) fn node_fn(node: &dyn Node, node_path: &[node::Id], conf: &NodeConf) -> ExprKind {
    // The binding used to receive the node's state as an argument, and whose
    // resulting value is returned from the body of the function and used to
    // update the state map.
    const STATE: &str = "state";

    fn input_name(i: usize) -> String {
        format!("input{i}")
    }

    // Create function parameters for graph state and inputs
    let mut input_args = conf
        .inputs
        .iter()
        .enumerate()
        .filter_map(|(i, b)| b.then(|| input_name(i)))
        .collect::<Vec<_>>();

    // Create input expressions for the node's expr method
    let input_exprs: Vec<Option<String>> = conf
        .inputs
        .iter()
        .enumerate()
        .map(|(i, b)| b.then(|| input_name(i)))
        .collect();

    // Get the node's expression
    let ctx = node::ExprCtx::new(node_path, &input_exprs, &conf.outputs);
    let node_expr = node.expr(ctx);

    // Construct the full function definition
    let fn_name = node_fn_name(node_path, &conf.inputs, &conf.outputs);
    let fn_body = if node.stateful() {
        input_args.push(STATE.to_string());
        format!("(let ((output {node_expr})) (list output state))")
    } else {
        format!("{node_expr}")
    };
    let fn_args = input_args.join(" ");
    let fn_def = format!("(define ({fn_name} {fn_args}) {fn_body})");

    Engine::emit_ast(&fn_def)
        .expect("Failed to emit AST for node function")
        .into_iter()
        .next()
        .unwrap()
}

/// Given a gantz graph and a rose tree with the associated node configs,
/// produce a function for every node configuration in the graph.
pub(crate) fn node_fns<G>(g: G, node_confs_tree: &RoseTree<NodeConfs>) -> Vec<ExprKind>
where
    G: Data<EdgeWeight = Edge> + IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable,
    G::NodeWeight: Node,
{
    let mut node_fns = NodeFns::new(&node_confs_tree);
    crate::graph::visit(g, &[], &mut node_fns);
    node_fns.fns
}
