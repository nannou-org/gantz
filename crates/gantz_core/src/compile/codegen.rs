//! Items related to lowering the control `Flow` graph to steel code.

use crate::{
    GRAPH_STATE, ROOT_STATE,
    compile::{Block, Flow, FlowGraph, Meta, MetaGraph, NodeConns, RoseTree},
    node,
};
pub(crate) use node_fn::{node_fns, unique_node_confs};
use petgraph::{
    graph::NodeIndex,
    visit::{EdgeRef, IntoNodeReferences, NodeRef},
};
use std::collections::{BTreeSet, HashSet};
use steel::{parser::ast::ExprKind, steel_vm::engine::Engine};

mod node_fn;

/// Specifies whether the argument for a node fn call comes directly from
/// another node output, or if it has a dedicated binding name.
#[derive(Clone, Debug)]
enum NodeFnCallArg {
    /// The arg comes directly from another node's output.
    /// E.g. `node-4-o2`.
    Output(node::Id, node::Output),
    /// The arg has a dedicated name (named after the node input) for
    /// disambiguation as the node is a part of a "join" in the flow graph.
    /// E.g. `node-6-i2`.
    DedicatedBinding,
}

/// Binding used for `state` local to each node fn.
const STATE: &str = "state";
/// Binding used for the current branch index.
const BRANCH_IX: &str = "branch-ix";

/// Whether or not the given node input requires a dedicated binding name.
///
/// This is required in the case that the node is within or following a "join"
/// control flow node, and the input has more than one incoming edge.
///
/// Disambiguation only requires generating one extra binding, so we just check
/// if there's more than one edge and don't worry about checking the flow graph.
fn node_input_needs_binding(mg: &MetaGraph, n: node::Id, input: usize) -> bool {
    let mut count = 0;
    for e_ref in mg.edges_directed(n, petgraph::Incoming) {
        for (edge, _kind) in e_ref.weight() {
            let ix = edge.input.0 as usize;
            count += (ix == input).then_some(1).unwrap_or(0);
            if count > 1 {
                return true;
            }
        }
    }
    false
}

/// The expression for a call to a node function.
fn node_fn_call(
    node_path: &[node::Id],
    inputs: &[Option<String>],
    outputs: &node::Conns,
    stateful: bool,
) -> ExprKind {
    // Prepare function arguments.
    let mut args: Vec<String> = inputs.iter().filter_map(Clone::clone).collect();
    if stateful {
        args.push(STATE.to_string());
    }

    // The expression for the node function call.
    let node_inputs = node::Conns::try_from_iter(inputs.iter().map(|arg| arg.is_some())).unwrap();
    let node_fn_name = node_fn::name(&node_path, &node_inputs, outputs);
    let node_fn_call_expr_str = format!("({node_fn_name} {})", args.join(" "));
    Engine::emit_ast(&node_fn_call_expr_str)
        .expect("failed to emit AST")
        .into_iter()
        .next()
        .unwrap()
}

/// Function to generate node input variable names for disambiguation.
fn node_input_var(node_ix: node::Id, in_ix: usize) -> String {
    format!("node-{}-i{}", node_ix, in_ix)
}

/// Function to generate node output variable names.
fn node_output_var(node_ix: node::Id, out_ix: usize) -> String {
    format!("node-{}-o{}", node_ix, out_ix)
}

/// The binding name given to the node's output.
fn node_outputs_var(node_ix: node::Id) -> String {
    format!("node-{node_ix}")
}

/// Find the arguments for a call to the node with the given ID with the given
/// connected inputs.
fn node_fn_call_args(
    mg: &MetaGraph,
    reachable: &HashSet<node::Id>,
    n: node::Id,
    inputs: &node::Conns,
) -> Vec<Option<NodeFnCallArg>> {
    let mut args = vec![None; inputs.len()];
    for e_ref in mg.edges_directed(n, petgraph::Incoming) {
        for (edge, _kind) in e_ref.weight() {
            let input_ix = edge.input.0 as usize;
            if inputs.get(input_ix).unwrap() && reachable.contains(&e_ref.source()) {
                args[input_ix] = match args[input_ix] {
                    // If there's no arg for this index yet, assign the output.
                    None => Some(NodeFnCallArg::Output(e_ref.source(), edge.output)),
                    // Otherwise if there's already an arg, that means there's
                    // more than one node output connected to this input
                    // (potentially from different branches) so this arg will
                    // have a dedicated binding name.
                    Some(_) => Some(NodeFnCallArg::DedicatedBinding),
                };
            }
        }
    }
    // Sanity check the connected inputs all have an arg.
    assert_eq!(
        inputs.iter().filter(|&b| b).count(),
        args.iter().filter(|opt| opt.is_some()).count(),
    );
    args
}

/// A statement within a sequence of statements for a top-level entrypoint or
/// nested graph function.
///
/// ### Parameters
///
/// - `node_path`: the node's nesting path relative to the root graph.
/// - `inputs`: the names of the output bindings that are being provided as
///   arguments to the node inputs.
///
/// Returns the statement.
fn eval_stmt(
    mg: &MetaGraph,
    reachable: &HashSet<node::Id>,
    node_path: &[node::Id],
    inputs: &node::Conns,
    outputs: &node::Conns,
    stateful: bool,
) -> ExprKind {
    // An expression for a node's key into the graph state hashmap.
    fn node_state_key(node_id: usize) -> ExprKind {
        // Create a symbol or other hashable key to use in the hashmap
        let key_str = format!("'{node_id}");
        Engine::emit_ast(&key_str)
            .expect("failed to emit AST")
            .into_iter()
            .next()
            .unwrap()
    }

    // Given a node's function call expression, wrap it in an expression
    // that provides access to its state.
    fn wrap_node_fn_call_with_state(call_expr: &str, node_ix: usize) -> String {
        const NEWSTATE: &str = "newstate";
        const OUTPUT: &str = "output";
        // Get the node's state key.
        let key = node_state_key(node_ix);
        format!(
            "(let (({STATE} (hash-ref {GRAPH_STATE} {key})))
               (let ((results {call_expr}))
                 (let (({OUTPUT} (car results)) ({NEWSTATE} (car (cdr results))))
                    (set! {GRAPH_STATE} (hash-insert {GRAPH_STATE} {key} {NEWSTATE}))
                    {OUTPUT})))"
        )
    }

    // Helper to create a define expression for a single output.
    // E.g. (define foo expr)
    fn create_define_expr(var_name: String, value_expr: ExprKind) -> ExprKind {
        let s = format!("(define {var_name} {})", value_expr);
        Engine::emit_ast(&s)
            .expect("failed to emit AST")
            .into_iter()
            .next()
            .unwrap()
    }

    // The node index is the last element of the path.
    let node_ix = *node_path.last().expect("node_path must not be empty");

    // Determine the input arg names.
    let input_args: Vec<_> = node_fn_call_args(mg, reachable, node_ix, inputs)
        .into_iter()
        .enumerate()
        .map(|(ix, src_opt)| {
            src_opt.as_ref().map(|arg| match arg {
                NodeFnCallArg::Output(n, out) => node_output_var(*n, out.0.into()),
                NodeFnCallArg::DedicatedBinding => node_input_var(node_ix, ix),
            })
        })
        .collect();

    // The expression for the node function call.
    let node_fn_call_expr = node_fn_call(node_path, &input_args, outputs, stateful);
    let mut node_fn_call_expr_str = format!("{node_fn_call_expr}");

    // Create the expression for the node.
    if stateful {
        node_fn_call_expr_str = wrap_node_fn_call_with_state(&node_fn_call_expr_str, node_ix);
    };
    let node_fn_call_expr = Engine::emit_ast(&node_fn_call_expr_str)
        .expect("failed to emit AST")
        .into_iter()
        .next()
        .unwrap();

    // Create a binding statement for the node's output.
    let stmt = match outputs.len() {
        0 => node_fn_call_expr,
        _ => {
            let output_var = node_outputs_var(node_ix);
            let define_expr = create_define_expr(output_var, node_fn_call_expr);
            define_expr
        }
    };

    stmt
}

/// Create a statement that binds a var for each value in the node's outputs.
fn destructure_node_outputs_stmt(n: node::Id, outputs: node::Conns) -> ExprKind {
    // Collect the names of the outputs.
    let vars: Vec<_> = outputs
        .iter()
        .enumerate()
        .filter_map(|(ix, b)| b.then(|| node_output_var(n, ix)))
        .collect();
    let outputs_var = node_outputs_var(n);
    let stmt = match vars.len() {
        1 => format!("(define {} {outputs_var})", vars.join(" ")),
        _ => format!("(define-values ({}) {outputs_var})", vars.join(" ")),
    };
    Engine::emit_ast(&stmt)
        .expect("failed to emit AST")
        .into_iter()
        .next()
        .unwrap()
}

/// Create a statement that destructures the node
fn destructure_node_branch_stmt(n: node::Id) -> ExprKind {
    let outputs_var = node_outputs_var(n);
    let stmt = format!("(define-values (branch-ix {outputs_var}) {outputs_var})");
    Engine::emit_ast(&stmt)
        .expect("failed to emit AST")
        .into_iter()
        .next()
        .unwrap()
}

/// Generate a function for performing evaluation of the given statements.
///
/// The given `Vec<ExprKind>` should be generated via the `eval_stmts` function.
fn eval_fn(eval_fn_name: &str, stmts: Vec<ExprKind>) -> ExprKind {
    // Create the body of the function as a sequence of expressions
    let stmts_str = stmts
        .iter()
        .map(|stmt| stmt.to_string())
        // `begin` block must end with a value, so we pass empty list.
        .chain(Some("'()".to_string()))
        .collect::<Vec<_>>()
        .join(" ");

    // Construct the full function definition
    let fn_def = format!(
        "(define ({}) \
           (define {GRAPH_STATE} {ROOT_STATE}) \
           {stmts_str} \
           (set! {ROOT_STATE} {GRAPH_STATE}))",
        eval_fn_name
    );

    // Parse the function definition into Steel AST
    Engine::emit_ast(&fn_def)
        .expect("Failed to emit AST for function")
        .into_iter()
        .next()
        .unwrap()
}

/// The string used to represent a path in a fn name.
fn path_string(path: &[node::Id]) -> String {
    path.iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(":")
}

/// The name used for the pull evaluation fn generated for the given node.
pub fn pull_eval_fn_name(path: &[node::Id]) -> String {
    format!("pull-fn-{}", path_string(path))
}

/// The name used for the push evaluation fn generated for the given node.
pub fn push_eval_fn_name(path: &[node::Id]) -> String {
    format!("push-fn-{}", path_string(path))
}

/// The set of outputs on the given node that require dedicated bindings due to
/// being connected to an input on a node that has multiple incoming edges.
fn node_outputs_that_need_bindings(
    mg: &MetaGraph,
    n: node::Id,
) -> BTreeSet<(node::Output, (node::Id, node::Input))> {
    let mut need_bindings = BTreeSet::default();
    for e_ref in mg.edges_directed(n, petgraph::Outgoing) {
        for (edge, _kind) in e_ref.weight() {
            if node_input_needs_binding(mg, e_ref.target(), edge.input.0 as usize) {
                need_bindings.insert((edge.output, (e_ref.target(), edge.input)));
            }
        }
    }
    need_bindings
}

/// Generate a statement that creates a node input binding for `dst` to the
/// given `src` node's output.
fn define_node_input_binding(
    (src, src_out): (node::Id, node::Output),
    (dst, dst_in): (node::Id, node::Input),
) -> ExprKind {
    let s = format!(
        "(define {} {})",
        node_input_var(dst, dst_in.0 as usize),
        node_output_var(src, src_out.0 as usize),
    );
    Engine::emit_ast(&s)
        .expect("failed to emit AST")
        .into_iter()
        .next()
        .unwrap()
}

// For the given node's outputs that connect to node inputs that have more than
// one incoming edge, create dedicated bindings for those inputs.
fn define_necessary_node_input_bindings(
    g: &MetaGraph,
    n: node::Id,
) -> impl Iterator<Item = ExprKind> {
    node_outputs_that_need_bindings(g, n)
        .into_iter()
        .map(move |(output, dst)| define_node_input_binding((n, output), dst))
}

/// Generate a sequence of node fn call statements from the given flow graph
/// basic block.
pub(crate) fn eval_fn_block_stmts(
    path: &[node::Id],
    mg: &MetaGraph,
    stateful: &BTreeSet<node::Id>,
    reachable: &HashSet<node::Id>,
    block: &Block,
) -> Vec<ExprKind> {
    let mut stmts = Vec::new();
    let mut iter = block.0.iter().peekable();
    while let Some(conf) = iter.next() {
        let node_path: Vec<_> = path.iter().copied().chain(Some(conf.id)).collect();
        let stateful = stateful.contains(&conf.id);
        let NodeConns { inputs, outputs } = conf.conns;
        let stmt = eval_stmt(mg, reachable, &node_path, &inputs, &outputs, stateful);
        stmts.push(stmt);

        // If this is the last node fn call in the block, it must be either
        // branching or terminal, so there's no need to destructure here.
        if iter.peek().is_none() {
            continue;
        }

        // Destructure the node's outputs.
        stmts.push(destructure_node_outputs_stmt(conf.id, conf.conns.outputs));
        // For outputs connected to node inputs that have more than one incoming
        // edge, we create dedicated bindings for those inputs.
        stmts.extend(define_necessary_node_input_bindings(mg, conf.id));
    }
    stmts
}

/// Find the entrypoint to the flow graph.
fn flow_graph_entry(fg: &FlowGraph) -> Option<NodeIndex<u32>> {
    let mut iter = fg
        .node_references()
        .map(|n_ref| n_ref.id())
        .filter(|&n| fg.edges_directed(n, petgraph::Incoming).next().is_none());
    let entry = iter.next();
    assert!(
        iter.next().is_none(),
        "flow graph should have only one entry"
    );
    entry
}

/// The set of unique nodes in the flow graph.
fn flow_graph_nodes(fg: &FlowGraph) -> HashSet<node::Id> {
    let mut set = HashSet::new();
    for n_ref in fg.node_references() {
        set.extend(n_ref.weight().iter().map(|conf| conf.id));
    }
    set
}

/// The eval fn statements for the control flow block at the given `flow_ix`.
fn flow_node_stmts(
    path: &[node::Id],
    flow_ix: NodeIndex,
    mg: &MetaGraph,
    stateful: &BTreeSet<node::Id>,
    reachable: &HashSet<node::Id>,
    fg: &FlowGraph,
) -> Vec<ExprKind> {
    // Add the block.
    let block = &fg[flow_ix];
    let mut stmts = eval_fn_block_stmts(path, mg, stateful, &reachable, block);

    // Collect the output branching edges.
    let mut edges: Vec<_> = fg
        .edges_directed(flow_ix, petgraph::Outgoing)
        .map(|e_ref| (*e_ref.weight(), e_ref.target()))
        .collect();
    edges.sort();

    // If there are no edges, we're done.
    if edges.is_empty() {
        return stmts;

    // If there is one edge, this must be part of a join.
    // FIXME: For now, just continue generating stmts. We should handle joins
    // properly though.
    } else if edges.len() == 1 {
        let (_branch, dst) = edges.pop().unwrap();

        // Destructure the last node's output.
        let conf = *block.last().unwrap();
        stmts.push(destructure_node_outputs_stmt(conf.id, conf.conns.outputs));
        stmts.extend(define_necessary_node_input_bindings(mg, conf.id));

        // Continue generating...
        stmts.extend(flow_node_stmts(path, dst, mg, stateful, reachable, fg));
        return stmts;
    }

    // Otherwise, add a statement to destructure the branch index.
    let conf = *block.last().unwrap();
    stmts.push(destructure_node_branch_stmt(conf.id));
    stmts.extend(define_necessary_node_input_bindings(mg, conf.id));

    // Add the branches.
    // FIXME: This doesn't properly handle joins.
    let mut expr = "'()".to_string();
    while let Some((branch, dst)) = edges.pop() {
        let dst_stmts = flow_node_stmts(path, dst, mg, stateful, reachable, fg);
        let dst_expr = format!(
            "(begin {} {} '())",
            destructure_node_outputs_stmt(conf.id, branch.conns),
            dst_stmts
                .into_iter()
                .map(|expr| format!("{expr}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        expr = format!("(if (= {} {BRANCH_IX}) {dst_expr} {expr})", branch.ix);
    }

    let expr = Engine::emit_ast(&expr)
        .expect("Failed to emit AST for function")
        .into_iter()
        .next()
        .unwrap();

    stmts.push(expr);
    stmts
}

/// Given the flow graph for an entry point eval fn, generate the body for the
/// fn. as a list of statements.
pub fn eval_fn_body(
    path: &[node::Id],
    mg: &MetaGraph,
    stateful: &BTreeSet<node::Id>,
    fg: &FlowGraph,
) -> Vec<ExprKind> {
    // Find the entry node. Collect the set of reachable nodes to filter out
    // unreachable nodes from the meta graph.
    let Some(entry) = flow_graph_entry(fg) else {
        return vec![];
    };
    let reachable = flow_graph_nodes(fg);
    flow_node_stmts(path, entry.id(), mg, stateful, &reachable, fg)
}

//// Generate all push and pull fns for the given control flow graph.
pub(crate) fn eval_fns_from_flow(
    path: &[node::Id],
    mg: &MetaGraph,
    stateful: &BTreeSet<node::Id>,
    flow: &Flow,
) -> Vec<ExprKind> {
    let pull_fgs = flow.pull.iter().map(|(&(id, _conns), fg)| {
        let node_path: Vec<_> = path.iter().copied().chain(Some(id)).collect();
        let name = pull_eval_fn_name(&node_path);
        (name, fg)
    });
    let push_fgs = flow.push.iter().map(|(&(id, _conns), fg)| {
        let node_path: Vec<_> = path.iter().copied().chain(Some(id)).collect();
        let name = push_eval_fn_name(&node_path);
        (name, fg)
    });
    pull_fgs
        .chain(push_fgs)
        .map(|(name, fg)| {
            let stmts = eval_fn_body(path, mg, stateful, fg);
            eval_fn(&name, stmts)
        })
        .collect()
}

/// Given a tree of eval plans for a gantz graph (and its nested graphs),
/// generate all push, pull and nested eval fns for the graph.
pub(crate) fn eval_fns(flow_tree: &RoseTree<(&Meta, Flow)>) -> Vec<ExprKind> {
    let mut eval_fns = vec![];
    flow_tree.visit(&[], &mut |path, (meta, flow)| {
        eval_fns.extend(eval_fns_from_flow(path, &meta.graph, &meta.stateful, flow));
    });
    eval_fns
}
