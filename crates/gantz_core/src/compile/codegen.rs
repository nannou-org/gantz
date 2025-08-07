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

/// Binding used for `state` local to each node fn.
const STATE: &str = "state";

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

/// Function to generate node output variable names.
fn output_var_name(node_ix: node::Id, out_ix: u16) -> String {
    format!("node-{}-o{}", node_ix, out_ix)
}

/// The names of the arguments for a call to the node with the given ID with the
/// given connected inputs.
fn node_fn_call_arg_srcs(
    g: &MetaGraph,
    reachable: &HashSet<node::Id>,
    n: node::Id,
    inputs: &node::Conns,
) -> Vec<Option<(node::Id, node::Output)>> {
    let mut args = vec![None; inputs.len()];
    for e_ref in g.edges_directed(n, petgraph::Incoming) {
        for (edge, _kind) in e_ref.weight() {
            if inputs.get(edge.input.0 as usize).unwrap() && reachable.contains(&e_ref.source()) {
                args[edge.input.0 as usize] = Some((e_ref.source(), edge.output));
            }
        }
    }
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

    // Helper to create a define-values expression for multiple outputs.
    // E.g. (define-values (foo bar) expr)
    fn create_define_values_expr(var_names: Vec<String>, value_expr: ExprKind) -> ExprKind {
        let s = format!("(define-values ({}) {})", var_names.join(" "), value_expr);
        Engine::emit_ast(&s)
            .expect("failed to emit AST")
            .into_iter()
            .next()
            .unwrap()
    }

    // The node index is the last element of the path.
    let node_ix = *node_path.last().expect("node_path must not be empty");

    // Determine the input arg names.
    let input_args: Vec<_> = node_fn_call_arg_srcs(mg, reachable, node_ix, inputs)
        .into_iter()
        .map(|src_opt| {
            src_opt
                .as_ref()
                .map(|&(node, output)| output_var_name(node, output.0))
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

    // Create a binding statement for each output
    let stmt = match outputs.len() {
        0 => node_fn_call_expr,
        1 => {
            let output_var = output_var_name(node_ix, 0);
            let define_expr = create_define_expr(output_var, node_fn_call_expr);
            define_expr
        }
        _ => {
            let output_vars: Vec<String> = (0..outputs.len())
                .map(|i| output_var_name(node_ix, i as u16))
                .collect();
            let define_values_expr = create_define_values_expr(output_vars, node_fn_call_expr);
            define_values_expr
        }
    };

    stmt
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
    for conf in &block.0 {
        let node_path: Vec<_> = path.iter().copied().chain(Some(conf.id)).collect();
        let stateful = stateful.contains(&conf.id);
        let NodeConns { inputs, outputs } = conf.conns;
        let stmt = eval_stmt(mg, reachable, &node_path, &inputs, &outputs, stateful);
        stmts.push(stmt);
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
    let entry = flow_graph_entry(fg).unwrap();
    let reachable = flow_graph_nodes(fg);

    // Walk the CFG depth-first generating all stmts for blocks and branches.
    // FIXME: do DFS manually and handle branches.
    let mut dfs = petgraph::visit::Dfs::new(fg, entry.id());
    let mut stmts = vec![];
    while let Some(n) = dfs.next(fg) {
        let block = &fg[n];
        stmts.extend(eval_fn_block_stmts(path, mg, stateful, &reachable, block));
    }
    stmts
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
