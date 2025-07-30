//! Items related to lowering the control `Flow` graph to steel code.

use crate::{
    GRAPH_STATE, ROOT_STATE,
    compile::{EvalPlan, EvalStep, RoseTree},
    node,
};
pub(crate) use node_fn::{node_confs_tree, node_fns};
use std::collections::{BTreeSet, HashMap};
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

/// A statement within a sequence of statements for a top-level entrypoint or
/// nested graph function.
///
/// ### Parameters
///
/// - `node_path`: the node's nesting path relative to the root graph.
/// - `inputs`: the names of the output bindings that are being provided as
///   arguments to the node inputs.
///
/// Returns the statement, alongside the name(s) of the output binding(s).
fn eval_stmt(
    node_path: &[node::Id],
    inputs: &[Option<String>],
    outputs: &node::Conns,
    stateful: bool,
) -> (ExprKind, Vec<String>) {
    // Function to generate variable names
    fn var_name(node_ix: node::Id, out_ix: u16) -> String {
        format!("node-{}-o{}", node_ix, out_ix)
    }

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

    // Create variables for this node's outputs
    let output_vars: Vec<_> = (0..outputs.len())
        .map(|i| var_name(node_ix, i as u16))
        .collect();

    // The expression for the node function call.
    let node_fn_call_expr = node_fn_call(node_path, inputs, outputs, stateful);
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
            let output_var = var_name(node_ix, 0);
            let define_expr = create_define_expr(output_var, node_fn_call_expr);
            define_expr
        }
        _ => {
            let output_vars: Vec<String> = (0..outputs.len())
                .map(|i| var_name(node_ix, i as u16))
                .collect();
            let define_values_expr = create_define_values_expr(output_vars, node_fn_call_expr);
            define_values_expr
        }
    };

    (stmt, output_vars)
}

/// Generate a sequence of evaluation statements for an eval function, one
/// statement for each given evaluation step.
///
/// The given `path`, `outputs` and `stateful` are associated with the graph
/// in which the eval fn is invoked.
pub(crate) fn eval_stmts(
    path: &[node::Id],
    steps: &[EvalStep],
    stateful: &BTreeSet<node::Id>,
) -> Vec<ExprKind> {
    // Track output variables
    let mut output_vars: HashMap<(node::Id, node::Output), String> = HashMap::new();
    let mut stmts = Vec::new();
    for step in steps {
        // Prepare input expressions for this node
        let inputs: Vec<_> = step
            .inputs
            .iter()
            .map(|arg_opt| {
                arg_opt.as_ref().map(|arg| {
                    let var_name = output_vars.get(&(arg.node, arg.output)).unwrap();
                    var_name.to_string()
                })
            })
            .collect();

        let node_path: Vec<_> = path.iter().copied().chain(Some(step.node)).collect();
        let stateful = stateful.contains(&step.node);

        // Produce the statement.
        let outputs = node::Conns::try_from_slice(&step.outputs).unwrap();
        let (stmt, stmt_outputs) = eval_stmt(&node_path, &inputs, &outputs, stateful);

        // Keep track of the output bindings.
        for (out_ix, out_var) in stmt_outputs.into_iter().enumerate() {
            let output = node::Output(out_ix.try_into().unwrap());
            output_vars.insert((step.node, output), out_var);
        }

        stmts.push(stmt);
    }
    stmts
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

/// Given a tree of eval plans for a gantz graph (and its nested graphs),
/// generate all push, pull and nested eval fns for the graph.
pub(crate) fn eval_fns(eval_tree: &RoseTree<EvalPlan>) -> Vec<ExprKind> {
    let mut eval_fns = vec![];
    eval_tree.visit(&[], &mut |path, eval| {
        let pull_steps = eval.pull_steps.iter().map(|(&id, steps)| {
            let node_path: Vec<_> = path.iter().copied().chain(Some(id)).collect();
            let name = pull_eval_fn_name(&node_path);
            (name, steps)
        });
        let push_steps = eval.push_steps.iter().map(|(&id, steps)| {
            let node_path: Vec<_> = path.iter().copied().chain(Some(id)).collect();
            let name = push_eval_fn_name(&node_path);
            (name, steps)
        });
        let fns = pull_steps.chain(push_steps).map(|(name, steps)| {
            let stmts = eval_stmts(path, &steps, &eval.meta.stateful);
            eval_fn(&name, stmts)
        });
        eval_fns.extend(fns);
    });
    eval_fns
}
