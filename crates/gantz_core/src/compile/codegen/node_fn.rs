//! Items related to the generation of node functions.
//!
//! In gantz, a function is generated for every unique configuration of every
//! node. That is, for each unique set of connected inputs and outputs of a
//! node, a function is generated.
//!
//! These configurations are collected by traversing from each of the push/pull
//! evaluation entrypoints.

use crate::{
    Edge,
    compile::{Flow, NodeConf, NodeConns, RoseTree, codegen::path_string},
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
type NodeConfs = BTreeSet<NodeConf>;

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

impl<'pl, Env> Visitor<Env> for NodeFns<'pl> {
    // We use `visit_post` so that the nested are generated before parents.
    fn visit_post(&mut self, ctx: visit::Ctx<Env>, node: &dyn Node<Env>) {
        use std::ops::Bound::{Excluded, Included};
        let node_path = ctx.path();
        let plan_path = &node_path[..node_path.len() - 1];
        let tree = self.tree.tree(&plan_path).unwrap();
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
            self.fns
                .push(node_fn(ctx.env(), node, node_path, &conf.conns));
        }
    }
}

/// The set of unique node configurations appearing within all evaluation paths.
pub(crate) fn unique_node_confs(flow: &Flow) -> NodeConfs {
    let mut confs = BTreeSet::new();
    confs.extend(
        flow.push
            .values()
            .flat_map(|g| g.node_weights().flat_map(|blk| blk.iter().copied())),
    );
    confs.extend(
        flow.pull
            .values()
            .flat_map(|g| g.node_weights().flat_map(|blk| blk.iter().copied())),
    );
    confs.extend(
        flow.nested
            .node_weights()
            .flat_map(|blk| blk.iter().copied()),
    );
    confs
}

/// Generate a function name for a node based on its path in the graph.
///
/// E.g. `node_fn_0_1_2_i0101_o1100
pub(crate) fn name(node_path: &[node::Id], inputs: &node::Conns, outputs: &node::Conns) -> String {
    let path_string = path_string(node_path);
    let inputs_prefix = if inputs.is_empty() { "" } else { "-i" };
    let outputs_prefix = if outputs.is_empty() { "" } else { "-o" };
    let inputs_string = format!("{inputs}");
    let outputs_string = format!("{outputs}");
    format!("node-fn-{path_string}{inputs_prefix}{inputs_string}{outputs_prefix}{outputs_string}")
}

/// Generate a function for a single node with the given set of connected inputs.
pub(crate) fn node_fn<Env>(
    env: &Env,
    node: &dyn Node<Env>,
    node_path: &[node::Id],
    conns: &NodeConns,
) -> ExprKind {
    // The binding used to receive the node's state as an argument, and whose
    // resulting value is returned from the body of the function and used to
    // update the state map.
    const STATE: &str = "state";

    fn input_name(i: usize) -> String {
        format!("input{i}")
    }

    // Create function parameters for graph state and inputs
    let mut input_args = conns
        .inputs
        .iter()
        .enumerate()
        .filter_map(|(i, b)| b.then(|| input_name(i)))
        .collect::<Vec<_>>();

    // Create input expressions for the node's expr method
    let input_exprs: Vec<Option<String>> = conns
        .inputs
        .iter()
        .enumerate()
        .map(|(i, b)| b.then(|| input_name(i)))
        .collect();

    // Get the node's expression
    let ctx = node::ExprCtx::new(env, node_path, &input_exprs, &conns.outputs);
    let node_expr = node.expr(ctx);

    // Construct the full function definition
    // FIXME: Remove this when switching to `flow::NodeConf`.
    let fn_name = name(node_path, &conns.inputs, &conns.outputs);
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
pub(crate) fn node_fns<Env, G>(
    env: &Env,
    g: G,
    node_confs_tree: &RoseTree<NodeConfs>,
) -> Vec<ExprKind>
where
    G: Data<EdgeWeight = Edge> + IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable,
    G::NodeWeight: Node<Env>,
{
    let mut node_fns = NodeFns::new(&node_confs_tree);
    crate::graph::visit(env, g, &[], &mut node_fns);
    node_fns.fns
}
