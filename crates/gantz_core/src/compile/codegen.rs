//! Items related to lowering the control `Flow` graph to steel code.

use super::loops::{LoopInfo, LoopTable};
use crate::{
    GRAPH_STATE, ROOT_STATE,
    compile::{
        Block, Flow, FlowGraph, Meta, MetaGraph, NodeConns, RoseTree,
        error::{CodegenError, InvalidInputIndex, TooManyConns},
        flow_graph_roots,
    },
    node,
};
pub(crate) use node_fn::{node_fns, unique_node_confs};
use petgraph::{
    graph::NodeIndex,
    visit::{EdgeRef, IntoEdgeReferences, IntoNodeReferences, NodeRef},
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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

/// Whether a nested graph's outlets should record their activation.
///
/// `Tracked` makes each outlet also `set!` an `outlet-active-{id}` flag (in
/// addition to its value), which the external branch selector reads to pick
/// which branch fired. Only needed when the graph branches externally;
/// top-level and push-through evaluation use `Untracked`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutletActivity {
    Tracked,
    Untracked,
}

/// The expression for a call to a node function.
fn node_fn_call(
    node_path: &[node::Id],
    inputs: &[Option<String>],
    outputs: &node::Conns,
    stateful: bool,
) -> Result<ExprKind, TooManyConns> {
    // Prepare function arguments.
    let mut args: Vec<String> = inputs.iter().filter_map(Clone::clone).collect();
    if stateful {
        args.push(STATE.to_string());
    }

    // The expression for the node function call.
    let node_inputs = node::Conns::try_from_iter(inputs.iter().map(|arg| arg.is_some()))
        .map_err(|_| TooManyConns(inputs.len()))?;
    let node_fn_name = node_fn::name(&node_path, &node_inputs, outputs);
    let node_fn_call_expr_str = format!("({node_fn_name} {})", args.join(" "));
    Ok(Engine::emit_ast(&node_fn_call_expr_str)
        .expect("failed to emit AST")
        .into_iter()
        .next()
        .unwrap())
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

/// The hoisted binding holding an outlet's value within a nested graph body.
///
/// Declared once in the body's enclosing scope and `set!` wherever the outlet
/// is reached, so its value survives out of any branch arm that produced it.
fn outlet_var(outlet_id: node::Id) -> String {
    format!("outlet-{outlet_id}")
}

/// The hoisted boolean flag recording whether an outlet was reached.
///
/// Only used when the nested graph branches externally; the final branch
/// selector reads these flags to determine which branch was taken.
fn outlet_active_var(outlet_id: node::Id) -> String {
    format!("outlet-active-{outlet_id}")
}

/// The `(define outlet-{id} '())` declarations (plus `outlet-active-{id}` flags
/// when `branching`) that hoist a graph's outlet value vars, so the
/// `(set! outlet-{id} ...)` statements emitted by `eval_stmt` have a binding.
///
/// Shared by `wrap_outlet_bridge` (nested graphs, read by the parent) and
/// `entry_fn_body` (a root graph's own outlets, which are never read - so root
/// outlets are effectively ignored).
fn outlet_var_declarations(outlets: &BTreeSet<node::Id>, branching: bool) -> Vec<String> {
    let mut declares = Vec::new();
    for &id in outlets {
        declares.push(format!("(define {} '())", outlet_var(id)));
        if branching {
            declares.push(format!("(define {} #f)", outlet_active_var(id)));
        }
    }
    declares
}

/// The Steel expression yielding the outlet values shaped by `outlet_ids`: a
/// single raw value for one outlet, a `(list ...)` for several, or `'()` for
/// none. Each value is read from its hoisted `outlet-{id}` var.
pub(crate) fn outlet_values_expr(outlet_ids: &[node::Id]) -> String {
    match outlet_ids {
        [] => "'()".to_string(),
        [id] => outlet_var(*id),
        ids => {
            let values: Vec<_> = ids.iter().map(|&id| outlet_var(id)).collect();
            format!("(list {})", values.join(" "))
        }
    }
}

/// The Steel expression selecting `(list branch-ix value)` for an externally
/// branching nested graph.
///
/// Each distinct outlet-activation pattern has a unique integer signature
/// (outlet `i` contributes `2^i` when active); the runtime signature is built
/// from the hoisted `outlet-active-{id}` flags and matched against each
/// pattern's signature with a nested `if` (the last pattern is the exhaustive
/// fallthrough). Only primitive Steel forms are used, since the VM runs the base
/// engine without the `cond`/`and` prelude macros.
///
/// Reused by both `nested_expr` (node-style branching) and `wrap_outlet_bridge`
/// (branch-aware push-through-outlet propagation).
pub(crate) fn branch_selector(patterns: &[node::Conns], outlet_ids: &[node::Id]) -> String {
    // The value to return for a pattern, shaped by its active outlet count.
    let value = |conns: &node::Conns| -> String {
        let active: Vec<node::Id> = outlet_ids
            .iter()
            .enumerate()
            .filter_map(|(i, &id)| conns.get(i).unwrap_or(false).then_some(id))
            .collect();
        outlet_values_expr(&active)
    };
    // The unique signature of a pattern's active outlets.
    let signature = |conns: &node::Conns| -> u128 {
        (0..outlet_ids.len())
            .filter(|&i| conns.get(i).unwrap_or(false))
            .map(|i| 1u128 << i)
            .sum()
    };

    // The runtime signature, summed from the active flags.
    let terms: Vec<String> = outlet_ids
        .iter()
        .enumerate()
        .map(|(i, &id)| format!("(if {} {} 0)", outlet_active_var(id), 1u128 << i))
        .collect();
    let sig_expr = match terms.as_slice() {
        [] => "0".to_string(),
        [t] => t.clone(),
        _ => format!("(+ {})", terms.join(" ")),
    };

    // Nested `if` over patterns; the last is the exhaustive fallthrough.
    let last = patterns.len() - 1;
    let mut expr = format!("(list {last} {})", value(&patterns[last]));
    for k in (0..last).rev() {
        expr = format!(
            "(if (= __branch-sig {}) (list {k} {}) {expr})",
            signature(&patterns[k]),
            value(&patterns[k]),
        );
    }
    format!("(let ((__branch-sig {sig_expr})) {expr})")
}

/// The per-branching-node binding holding its selected branch index.
///
/// Declared once per body (see [`declare_branch_ix_placeholders`]) and `set!`
/// when the branch destructures, so it is readable from any arm scope without
/// colliding with sibling branches' selectors.
fn branch_ix_var(node_ix: node::Id) -> String {
    format!("branch-ix-{node_ix}")
}

/// `(define branch-ix-{id} -1)` placeholders for every branching node, declared
/// at the top of a body before any branch `set!`s them.
fn declare_branch_ix_placeholders(branching: &BTreeMap<node::Id, usize>) -> Vec<ExprKind> {
    branching
        .keys()
        .map(|&id| emit_one(&format!("(define {} -1)", branch_ix_var(id))))
        .collect()
}

/// Parse a single Steel statement string into its AST node.
fn emit_one(src: &str) -> ExprKind {
    Engine::emit_ast(src)
        .expect("failed to emit AST")
        .into_iter()
        .next()
        .unwrap()
}

/// Find the arguments for a call to the node with the given ID with the given
/// connected inputs.
fn node_fn_call_args(
    mg: &MetaGraph,
    reachable: &HashSet<node::Id>,
    n: node::Id,
    inputs: &node::Conns,
) -> Result<Vec<Option<NodeFnCallArg>>, InvalidInputIndex> {
    let mut args = vec![None; inputs.len()];
    for e_ref in mg.edges_directed(n, petgraph::Incoming) {
        for (edge, _kind) in e_ref.weight() {
            let input_ix = edge.input.0 as usize;
            let is_connected = inputs.get(input_ix).ok_or(InvalidInputIndex {
                index: input_ix,
                n_inputs: inputs.len(),
            })?;
            if is_connected && reachable.contains(&e_ref.source()) {
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
    Ok(args)
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
    inlets: &BTreeSet<node::Id>,
    outlets: &BTreeSet<node::Id>,
    node_path: &[node::Id],
    inputs: &node::Conns,
    outputs: &node::Conns,
    stateful: bool,
    outlet_activity: OutletActivity,
) -> Result<Option<ExprKind>, CodegenError> {
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

    // For inlet nodes, create a direct binding to the inlet value instead of
    // calling a node function. The inlet value is provided by the parent graph
    // via a `(define inlet-{ix} ...)` binding.
    if inlets.contains(&node_ix) {
        let inlet_var = format!("inlet-{node_ix}");
        let output_var = node_outputs_var(node_ix);
        return Ok(Some(create_define_expr(
            output_var,
            Engine::emit_ast(&inlet_var)
                .expect("failed to emit AST")
                .into_iter()
                .next()
                .unwrap(),
        )));
    }

    // Determine the input arg names (used for outlet hoisting and node calls).
    let input_args: Vec<_> = node_fn_call_args(mg, reachable, node_ix, inputs)?
        .into_iter()
        .enumerate()
        .map(|(ix, src_opt)| {
            src_opt.as_ref().map(|arg| match arg {
                NodeFnCallArg::Output(n, out) => node_output_var(*n, out.0.into()),
                NodeFnCallArg::DedicatedBinding => node_input_var(node_ix, ix),
            })
        })
        .collect();

    // For outlet nodes, set the hoisted `outlet-{id}` var (and, in branching
    // mode, its active flag) from the connected input rather than calling a node
    // function. Placed by the surrounding flow recursion within whatever branch
    // arm reaches the outlet; the parent reads the hoisted var. A disconnected
    // outlet leaves its var at the `'()` default.
    if outlets.contains(&node_ix) {
        let Some(src) = input_args.first().and_then(Option::as_ref) else {
            return Ok(None);
        };
        let set_value = format!("(set! {} {src})", outlet_var(node_ix));
        let s = if outlet_activity == OutletActivity::Tracked {
            format!(
                "(begin {set_value} (set! {} #t))",
                outlet_active_var(node_ix)
            )
        } else {
            set_value
        };
        return Ok(Some(emit_one(&s)));
    }

    // The expression for the node function call.
    let node_fn_call_expr = node_fn_call(node_path, &input_args, outputs, stateful)?;
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

    Ok(Some(stmt))
}

/// Create a statement that binds a var for each value in the node's outputs.
///
/// Returns `None` when the node has no connected outputs.
fn destructure_node_outputs_stmt(n: node::Id, outputs: node::Conns) -> Option<ExprKind> {
    // Collect the names of the outputs.
    let vars: Vec<_> = outputs
        .iter()
        .enumerate()
        .filter_map(|(ix, b)| b.then(|| node_output_var(n, ix)))
        .collect();
    if vars.is_empty() {
        return None;
    }
    let outputs_var = node_outputs_var(n);
    let stmt = match vars.len() {
        1 => format!("(define {} {outputs_var})", vars.join(" ")),
        _ => format!("(define-values ({}) {outputs_var})", vars.join(" ")),
    };
    Some(
        Engine::emit_ast(&stmt)
            .expect("failed to emit AST")
            .into_iter()
            .next()
            .unwrap(),
    )
}

/// Destructure a branching node's `(branch-ix value)` output: store the branch
/// index in its pre-declared `branch-ix-{n}` var and rebind the node's var to
/// the value. Uses `set!` (not a fresh `define`) so it works from any arm scope.
fn destructure_node_branch_stmt(n: node::Id) -> ExprKind {
    let outputs_var = node_outputs_var(n);
    let branch_ix = branch_ix_var(n);
    emit_one(&format!(
        "(begin (set! {branch_ix} (list-ref {outputs_var} 0)) \
               (set! {outputs_var} (list-ref {outputs_var} 1)))"
    ))
}

/// Generate a function for performing evaluation of the given statements.
///
/// The given `Vec<ExprKind>` should be generated via the `eval_stmts` function.
fn entry_fn(name: &str, stmts: Vec<ExprKind>) -> ExprKind {
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
        name
    );

    // Parse the function definition into Steel AST
    Engine::emit_ast(&fn_def)
        .expect("Failed to emit AST for function")
        .into_iter()
        .next()
        .unwrap()
}

/// The string used to represent a path in a fn name.
pub(crate) fn path_string(path: &[node::Id]) -> String {
    path.iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(":")
}

/// Generate entry fn name from an `EntrypointId`.
///
/// The name is deterministic and unique - derived from the content hash
/// (truncated to 8 hex chars).
pub fn entry_fn_name(id: &super::EntrypointId) -> String {
    format!("entry-fn-{}", id.0.display_short())
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

/// For the given target node, create dedicated input bindings for all inputs
/// that have multiple incoming MetaGraph edges (i.e. `DedicatedBinding` args).
///
/// Skips inputs in `active_phi` - those are handled by phi variables at join
/// points.
///
/// For single-source inputs, emits `(define node-X-iY node-SRC-oZ)`.
/// For multi-source inputs, emits `(define node-X-iY (list ...))`.
fn define_target_input_bindings(
    mg: &MetaGraph,
    reachable: &HashSet<node::Id>,
    in_scope: &HashSet<node::Id>,
    n: node::Id,
    conns: &NodeConns,
    active_phi: &HashSet<(node::Id, usize)>,
) -> Result<Vec<ExprKind>, CodegenError> {
    let args = node_fn_call_args(mg, reachable, n, &conns.inputs)?;
    let mut stmts = Vec::new();
    for (ix, arg) in args.iter().enumerate() {
        if !matches!(arg, Some(NodeFnCallArg::DedicatedBinding)) {
            continue;
        }
        if active_phi.contains(&(n, ix)) {
            continue;
        }
        // Collect sources whose outputs are currently in scope.
        let mut sources: Vec<(node::Id, node::Output)> = Vec::new();
        for e_ref in mg.edges_directed(n, petgraph::Incoming) {
            if !in_scope.contains(&e_ref.source()) {
                continue;
            }
            for (edge, _kind) in e_ref.weight() {
                if edge.input.0 as usize == ix {
                    sources.push((e_ref.source(), edge.output));
                }
            }
        }
        // Sort by node ID for deterministic ordering.
        sources.sort_by_key(|(src, _)| *src);
        match sources.len() {
            0 => {}
            1 => {
                let (src, src_out) = sources[0];
                stmts.push(define_node_input_binding(
                    (src, src_out),
                    (n, node::Input(ix as u16)),
                ));
            }
            _ => {
                let elements: Vec<String> = sources
                    .iter()
                    .map(|(src, out)| node_output_var(*src, out.0 as usize))
                    .collect();
                let s = format!(
                    "(define {} (list {}))",
                    node_input_var(n, ix),
                    elements.join(" "),
                );
                stmts.push(
                    Engine::emit_ast(&s)
                        .expect("failed to emit AST")
                        .into_iter()
                        .next()
                        .unwrap(),
                );
            }
        }
    }
    Ok(stmts)
}

/// For a join block's first node, find which inputs are `DedicatedBinding`
/// (have multiple incoming MetaGraph edges) and need phi variables.
///
/// Returns `(input_ix, phi_var_name)` pairs.
fn join_phi_params(
    mg: &MetaGraph,
    reachable: &HashSet<node::Id>,
    block: &Block,
) -> Result<Vec<(usize, String)>, CodegenError> {
    let first = block.first().expect("block must not be empty");
    let args = node_fn_call_args(mg, reachable, first.id, &first.conns.inputs)?;
    Ok(args
        .into_iter()
        .enumerate()
        .filter_map(|(ix, arg)| match arg {
            Some(NodeFnCallArg::DedicatedBinding) => Some((ix, node_input_var(first.id, ix))),
            _ => None,
        })
        .collect())
}

/// Generate `(define name '())` placeholder statements for phi variables.
fn declare_phi_vars(phi_params: &[(usize, String)]) -> Vec<ExprKind> {
    phi_params
        .iter()
        .map(|(_, name)| {
            let stmt = format!("(define {name} '())");
            Engine::emit_ast(&stmt)
                .expect("failed to emit AST")
                .into_iter()
                .next()
                .unwrap()
        })
        .collect()
}

/// Generate `(set! name value)` statements to assign phi variables from a
/// predecessor node's outputs.
///
/// When `active_outputs` is `Some`, only edges whose output index is active
/// in the given conns are considered. This is needed when the predecessor is
/// a branch node with multiple outputs feeding the same target input - each
/// arm must reference only its own active output.
fn phi_set_stmts(
    mg: &MetaGraph,
    pred_last_node: node::Id,
    join_node_id: node::Id,
    phi_params: &[(usize, String)],
    active_outputs: Option<node::Conns>,
) -> Vec<ExprKind> {
    // Collect edges from the predecessor to the join node, optionally
    // filtered to the branch arm's active outputs.
    let edges: Vec<_> = mg
        .edges_directed(join_node_id, petgraph::Incoming)
        .filter(|e| e.source() == pred_last_node)
        .flat_map(|e| e.weight().clone())
        .filter(|(edge, _kind)| {
            active_outputs.map_or(true, |conns| {
                conns.get(edge.output.0 as usize).unwrap_or(false)
            })
        })
        .collect();
    phi_params
        .iter()
        .filter_map(|(input_ix, name)| {
            edges.iter().find_map(|(edge, _kind)| {
                if edge.input.0 as usize == *input_ix {
                    let value = node_output_var(pred_last_node, edge.output.0 as usize);
                    let stmt = format!("(set! {name} {value})");
                    Some(
                        Engine::emit_ast(&stmt)
                            .expect("failed to emit AST")
                            .into_iter()
                            .next()
                            .unwrap(),
                    )
                } else {
                    None
                }
            })
        })
        .collect()
}

/// Generate a sequence of node fn call statements from the given flow graph
/// basic block.
pub(crate) fn eval_fn_block_stmts(
    path: &[node::Id],
    mg: &MetaGraph,
    stateful: &BTreeSet<node::Id>,
    inlets: &BTreeSet<node::Id>,
    outlets: &BTreeSet<node::Id>,
    reachable: &HashSet<node::Id>,
    block: &Block,
    in_scope: &mut HashSet<node::Id>,
    active_phi: &HashSet<(node::Id, usize)>,
    bridge_nodes: &BTreeSet<node::Id>,
    outlet_activity: OutletActivity,
) -> Result<Vec<ExprKind>, CodegenError> {
    let mut stmts = Vec::new();
    let mut iter = block.0.iter().peekable();
    while let Some(conf) = iter.next() {
        // Bridge nodes have their output already bound by an outlet bridge.
        // Skip the eval but still destructure outputs for downstream use.
        if bridge_nodes.contains(&conf.id) {
            if iter.peek().is_some() {
                stmts.extend(destructure_node_outputs_stmt(conf.id, conf.conns.outputs));
                in_scope.insert(conf.id);
            }
            continue;
        }

        // Before evaluating this node, create dedicated input bindings for
        // any inputs with multiple incoming edges (scalar or list).
        stmts.extend(define_target_input_bindings(
            mg,
            reachable,
            in_scope,
            conf.id,
            &conf.conns,
            active_phi,
        )?);

        let node_path: Vec<_> = path.iter().copied().chain(Some(conf.id)).collect();
        let stateful = stateful.contains(&conf.id);
        let NodeConns { inputs, outputs } = conf.conns;
        let Some(stmt) = eval_stmt(
            mg,
            reachable,
            inlets,
            outlets,
            &node_path,
            &inputs,
            &outputs,
            stateful,
            outlet_activity,
        )?
        else {
            continue;
        };
        stmts.push(stmt);

        // If this is the last node fn call in the block, it must be either
        // branching or terminal, so there's no need to destructure here.
        if iter.peek().is_none() {
            continue;
        }

        // Destructure the node's outputs and mark them as in-scope.
        stmts.extend(destructure_node_outputs_stmt(conf.id, conf.conns.outputs));
        in_scope.insert(conf.id);
    }
    Ok(stmts)
}

/// The set of unique nodes in the flow graph.
fn flow_graph_nodes(fg: &FlowGraph) -> HashSet<node::Id> {
    let mut set = HashSet::new();
    for n_ref in fg.node_references() {
        set.extend(n_ref.weight().iter().map(|conf| conf.id));
    }
    set
}

/// Find flow graph nodes that are join points (in-degree > 1).
fn find_join_points(fg: &FlowGraph) -> HashSet<NodeIndex> {
    fg.node_references()
        .filter(|n_ref| {
            fg.edges_directed(n_ref.id(), petgraph::Incoming)
                .take(2)
                .count()
                > 1
        })
        .map(|n_ref| n_ref.id())
        .collect()
}

/// Compute immediate post-dominators for all flow graph nodes.
///
/// The immediate post-dominator of a node is the first node that ALL
/// paths from it must pass through. For a branching block, this is the
/// reconvergence point.
///
/// Built by reversing the flow graph, adding a virtual exit, and
/// computing the dominator tree on the reversed graph.
fn compute_post_dominators(fg: &FlowGraph) -> HashMap<NodeIndex, NodeIndex> {
    use petgraph::algo::dominators;

    let nodes: Vec<_> = fg.node_indices().collect();
    if nodes.is_empty() {
        return HashMap::new();
    }

    // Build reversed graph (plain DiGraph for trait compat with simple_fast).
    let mut reversed = petgraph::graph::DiGraph::<(), ()>::new();
    let mut fg_to_rev = HashMap::new();
    for &n in &nodes {
        fg_to_rev.insert(n, reversed.add_node(()));
    }
    for e_ref in fg.edge_references() {
        reversed.add_edge(fg_to_rev[&e_ref.target()], fg_to_rev[&e_ref.source()], ());
    }

    // Virtual exit: root of reversed graph.
    // Terminal nodes (no outgoing edges in original) connect from this root.
    let virtual_exit = reversed.add_node(());
    for &n in &nodes {
        if fg.edges_directed(n, petgraph::Outgoing).count() == 0 {
            reversed.add_edge(virtual_exit, fg_to_rev[&n], ());
        }
    }

    let doms = dominators::simple_fast(&reversed, virtual_exit);

    // Map back: immediate dominator in reversed = post-dominator in original.
    let rev_to_fg: HashMap<_, _> = fg_to_rev.iter().map(|(&f, &r)| (r, f)).collect();
    let mut post_dom = HashMap::new();
    for &fg_n in &nodes {
        if let Some(idom) = doms.immediate_dominator(fg_to_rev[&fg_n]) {
            // Filter out virtual exit (not in rev_to_fg).
            if let Some(&orig) = rev_to_fg.get(&idom) {
                post_dom.insert(fg_n, orig);
            }
        }
    }
    post_dom
}

/// The eval fn statements for the control flow block at the given `flow_ix`.
///
/// When `stop_at` is `Some(join_ix)`, generation stops when reaching that
/// join - phi set statements are emitted and control returns to the caller
/// which handles the join block after its branch `if`.
fn flow_node_stmts(
    path: &[node::Id],
    flow_ix: NodeIndex,
    mg: &MetaGraph,
    stateful: &BTreeSet<node::Id>,
    branching: &BTreeMap<node::Id, usize>,
    inlets: &BTreeSet<node::Id>,
    outlets: &BTreeSet<node::Id>,
    reachable: &HashSet<node::Id>,
    fg: &FlowGraph,
    post_dom: &HashMap<NodeIndex, NodeIndex>,
    phi_params: &BTreeMap<NodeIndex, Vec<(usize, String)>>,
    in_scope: &mut HashSet<node::Id>,
    active_phi: &HashSet<(node::Id, usize)>,
    stop_at: Option<NodeIndex>,
    bridge_nodes: &BTreeSet<node::Id>,
    outlet_activity: OutletActivity,
    current_loop: Option<&LoopInfo>,
    loops: &LoopTable,
) -> Result<Vec<ExprKind>, CodegenError> {
    // A block that begins a feedback loop opens a tail-recursive loop fn - unless
    // we are already lowering that loop's own body (the body recursion re-enters
    // here on the header). A *nested* header (a different id) still opens its own
    // loop fn while inside the outer one.
    let first_id = fg[flow_ix]
        .first()
        .expect("flow block must not be empty")
        .id;
    if let Some(loop_info) = loops.get(&first_id) {
        let already_inside = current_loop.map_or(false, |l| l.header == first_id);
        if !already_inside {
            return flow_loop_stmts(
                path,
                flow_ix,
                loop_info,
                mg,
                stateful,
                branching,
                inlets,
                outlets,
                reachable,
                fg,
                post_dom,
                phi_params,
                in_scope,
                active_phi,
                stop_at,
                bridge_nodes,
                outlet_activity,
                current_loop,
                loops,
            );
        }
    }

    // Add the block.
    let block = &fg[flow_ix];
    let mut stmts = eval_fn_block_stmts(
        path,
        mg,
        stateful,
        inlets,
        outlets,
        reachable,
        block,
        in_scope,
        active_phi,
        bridge_nodes,
        outlet_activity,
    )?;

    // If this block ends in the current loop's deciding branch, emit the loop
    // decision (continue arm -> tail-call, exit arm -> return value) and stop;
    // the exit downstream is lowered by the loop wrapper after the call.
    if let Some(loop_info) = current_loop {
        let last = *block.last().expect("flow block must not be empty");
        if loop_info.continue_arms.contains_key(&last.id) {
            stmts.push(destructure_node_branch_stmt(last.id));
            stmts.push(loop_decision_if(loop_info, last.id, last.conns.outputs));
            return Ok(stmts);
        }
    }

    // Collect the output branching edges.
    let mut edges: Vec<_> = fg
        .edges_directed(flow_ix, petgraph::Outgoing)
        .map(|e_ref| (*e_ref.weight(), e_ref.target()))
        .collect();
    edges.sort();

    // If there are no edges, we're done. Destructure the block's last node so a
    // consumer in another flow component - e.g. a branch's reconvergence join on
    // a different root, emitted later after `order_roots` - can reference its
    // outputs. (Within a component, a downstream node would do this; a terminal
    // node otherwise leaves them undefined.) A terminal branch is left alone.
    if edges.is_empty() {
        let conf = *block.last().unwrap();
        if !branching.contains_key(&conf.id) {
            stmts.extend(destructure_node_outputs_stmt(conf.id, conf.conns.outputs));
            in_scope.insert(conf.id);
        }
        return Ok(stmts);

    // Single successor: either a linear continuation or leads to a join.
    } else if edges.len() == 1 && !branching.contains_key(&block.last().unwrap().id) {
        let (_branch, dst) = edges.pop().unwrap();
        let conf = *block.last().unwrap();
        stmts.extend(destructure_node_outputs_stmt(conf.id, conf.conns.outputs));
        in_scope.insert(conf.id);

        // If the successor is the reconvergence point we're stopping at,
        // emit phi set statements and return.
        if stop_at == Some(dst) {
            let join_first_id = fg[dst].first().expect("join block must not be empty").id;
            if let Some(params) = phi_params.get(&dst) {
                stmts.extend(phi_set_stmts(mg, conf.id, join_first_id, params, None));
            }
            return Ok(stmts);
        }

        // Otherwise continue normally. The next block's
        // `eval_fn_block_stmts` handles input bindings via before-target.
        stmts.extend(flow_node_stmts(
            path,
            dst,
            mg,
            stateful,
            branching,
            inlets,
            outlets,
            reachable,
            fg,
            post_dom,
            phi_params,
            in_scope,
            active_phi,
            stop_at,
            bridge_nodes,
            outlet_activity,
            current_loop,
            loops,
        )?);
        return Ok(stmts);
    }

    // Multiple edges: this is a branch.
    let conf = *block.last().unwrap();
    stmts.push(destructure_node_branch_stmt(conf.id));

    // Post-dominator gives the reconvergence point directly.
    // Skip if it equals stop_at (handled by outer level).
    // Also skip if this branching node has dead branches (fewer edges than
    // branches) - the join block must not run unconditionally when some
    // branches terminate evaluation.
    let reconvergence = post_dom
        .get(&flow_ix)
        .copied()
        .filter(|&r| stop_at != Some(r))
        .filter(|_| {
            branching
                .get(&conf.id)
                .map_or(true, |&n_branches| edges.len() >= n_branches)
        });

    if let Some(join_ix) = reconvergence {
        // Declare phi variable placeholders for the join's inputs.
        if let Some(params) = phi_params.get(&join_ix) {
            stmts.extend(declare_phi_vars(params));
        }

        // Extend active phi vars with this join's phi params so that
        // branch bodies and deeper recursions won't shadow them.
        let mut inner_phi = active_phi.clone();
        if let Some(params) = phi_params.get(&join_ix) {
            let join_first_id = fg[join_ix].first().expect("block must not be empty").id;
            for (ix, _) in params {
                inner_phi.insert((join_first_id, *ix));
            }
        }

        // Build nested if-else with each branch body.
        let mut expr = "'()".to_string();
        let mut sorted_edges = edges;
        while let Some((branch, dst)) = sorted_edges.pop() {
            let mut branch_stmts: Vec<ExprKind> =
                destructure_node_outputs_stmt(conf.id, branch.conns)
                    .into_iter()
                    .collect();

            if dst == join_ix {
                // Branch target IS the join - just emit phi sets.
                let join_first_id = fg[join_ix]
                    .first()
                    .expect("join block must not be empty")
                    .id;
                if let Some(params) = phi_params.get(&join_ix) {
                    branch_stmts.extend(phi_set_stmts(
                        mg,
                        conf.id,
                        join_first_id,
                        params,
                        Some(branch.conns),
                    ));
                }
            } else {
                // Recurse with stop_at = join_ix so it stops at the join.
                // Clone in_scope so this arm doesn't leak its nodes to
                // sibling arms. Add the branch node since its outputs were
                // destructured above.
                let mut arm_scope = in_scope.clone();
                arm_scope.insert(conf.id);
                branch_stmts.extend(flow_node_stmts(
                    path,
                    dst,
                    mg,
                    stateful,
                    branching,
                    inlets,
                    outlets,
                    reachable,
                    fg,
                    post_dom,
                    phi_params,
                    &mut arm_scope,
                    &inner_phi,
                    Some(join_ix),
                    bridge_nodes,
                    outlet_activity,
                    current_loop,
                    loops,
                )?);
            }

            let branch_str = branch_stmts
                .iter()
                .map(|s| format!("{s}"))
                .collect::<Vec<_>>()
                .join(" ");
            expr = format!(
                "(if (= {} {}) (begin {branch_str}) {expr})",
                branch.ix,
                branch_ix_var(conf.id),
            );
        }

        let expr = Engine::emit_ast(&expr)
            .expect("failed to emit AST for branch")
            .into_iter()
            .next()
            .unwrap();
        stmts.push(expr);

        // After the if: generate the join block and everything after it.
        stmts.extend(flow_node_stmts(
            path,
            join_ix,
            mg,
            stateful,
            branching,
            inlets,
            outlets,
            reachable,
            fg,
            post_dom,
            phi_params,
            in_scope,
            &inner_phi,
            stop_at,
            bridge_nodes,
            outlet_activity,
            current_loop,
            loops,
        )?);

        return Ok(stmts);
    }

    // No reconvergence found - fall back to code duplication.
    let mut expr = "'()".to_string();
    while let Some((branch, dst)) = edges.pop() {
        // Clone in_scope so each arm is independent. Add the branch node
        // since its outputs are destructured in each arm below.
        let mut arm_scope = in_scope.clone();
        arm_scope.insert(conf.id);
        let dst_stmts = flow_node_stmts(
            path,
            dst,
            mg,
            stateful,
            branching,
            inlets,
            outlets,
            reachable,
            fg,
            post_dom,
            phi_params,
            &mut arm_scope,
            active_phi,
            stop_at,
            bridge_nodes,
            outlet_activity,
            current_loop,
            loops,
        )?;
        let dst_stmts_str = dst_stmts
            .into_iter()
            .map(|expr| format!("{expr}"))
            .collect::<Vec<_>>()
            .join(" ");
        let destructure_str = destructure_node_outputs_stmt(conf.id, branch.conns)
            .map(|e| format!("{e}"))
            .unwrap_or_default();
        let dst_expr = format!("(begin {} {} '())", destructure_str, dst_stmts_str,);
        expr = format!(
            "(if (= {} {}) {dst_expr} {expr})",
            branch.ix,
            branch_ix_var(conf.id),
        );
    }

    let expr = Engine::emit_ast(&expr)
        .expect("failed to emit AST for branch")
        .into_iter()
        .next()
        .unwrap();

    stmts.push(expr);
    Ok(stmts)
}

/// The name of the tail-recursive local fn generated for a loop with `header`.
fn loop_fn_name(header: node::Id) -> String {
    format!("loopfn-{header}")
}

/// The loop decision emitted when `flow_node_stmts` reaches a loop's deciding
/// branch: each continue arm tail-calls the loop fn with the fed-back value, any
/// other (exit) arm returns it. `node-{decide}` is the per-iteration value (the
/// branch's selected-arm value, after `destructure_node_branch_stmt`).
fn loop_decision_if(
    loop_info: &LoopInfo,
    decide_id: node::Id,
    decide_outputs: node::Conns,
) -> ExprKind {
    let loop_fn = loop_fn_name(loop_info.header);
    let branch_val = node_outputs_var(decide_id);
    let bix = branch_ix_var(decide_id);

    // The tail call passes one arg per loop-carried param, taken from the branch
    // output that drives that header input's back-edge.
    let tail_call = if loop_info.carried.len() <= 1 {
        // Single param: the branch value is the fed-back value directly.
        format!("({loop_fn} {branch_val})")
    } else {
        // Multiple params: the continue arm's value is a list shaped by the
        // back-edge outputs; destructure it into per-output vars, then pass the
        // arg feeding each param (in carried order).
        let back_by_input: BTreeMap<usize, usize> = loop_info
            .back_edges
            .iter()
            .filter(|(src, _)| *src == decide_id)
            .map(|(_, e)| (e.input.0 as usize, e.output.0 as usize))
            .collect();
        let mut continue_conns =
            node::Conns::unconnected(decide_outputs.len()).expect("conns len within bounds");
        for &o in back_by_input.values() {
            let _ = continue_conns.set(o, true);
        }
        let destructure = destructure_node_outputs_stmt(decide_id, continue_conns)
            .map(|e| format!("{e} "))
            .unwrap_or_default();
        let tail_args: Vec<String> = loop_info
            .carried
            .iter()
            .map(|p| node_output_var(decide_id, back_by_input[&p.header_input]))
            .collect();
        format!("(begin {destructure}({loop_fn} {}))", tail_args.join(" "))
    };

    let mut if_expr = branch_val.clone();
    for arm in &loop_info.continue_arms[&decide_id] {
        if_expr = format!("(if (= {arm} {bix}) {tail_call} {if_expr})");
    }
    emit_one(&if_expr)
}

/// Lower a feedback loop as a tail-recursive local fn, then continue with its
/// downstream (the deciding branch's exit arms, which consume the result).
///
/// The loop fn body spans the header block through the single deciding branch.
/// It is produced by recursing the ordinary lowering (`flow_node_stmts`) over the
/// body with this loop marked active, so any inner forward branches reuse the
/// existing reconvergence/phi machinery; when the recursion reaches the deciding
/// branch it emits the loop decision (see [`loop_decision_if`]) and stops. Each
/// loop-carried header input becomes a fn parameter named after the header's
/// dedicated input binding, so the header node-fn call is unchanged. The result
/// is bound to the deciding branch's outputs var so downstream is unchanged.
fn flow_loop_stmts(
    path: &[node::Id],
    flow_ix: NodeIndex,
    loop_info: &LoopInfo,
    mg: &MetaGraph,
    stateful: &BTreeSet<node::Id>,
    branching: &BTreeMap<node::Id, usize>,
    inlets: &BTreeSet<node::Id>,
    outlets: &BTreeSet<node::Id>,
    reachable: &HashSet<node::Id>,
    fg: &FlowGraph,
    post_dom: &HashMap<NodeIndex, NodeIndex>,
    phi_params: &BTreeMap<NodeIndex, Vec<(usize, String)>>,
    in_scope: &mut HashSet<node::Id>,
    active_phi: &HashSet<(node::Id, usize)>,
    stop_at: Option<NodeIndex>,
    bridge_nodes: &BTreeSet<node::Id>,
    outlet_activity: OutletActivity,
    current_loop: Option<&LoopInfo>,
    loops: &LoopTable,
) -> Result<Vec<ExprKind>, CodegenError> {
    let header = loop_info.header;
    // Analysis guarantees exactly one deciding branch.
    let decide_id = *loop_info
        .continue_arms
        .keys()
        .next()
        .expect("a loop has exactly one deciding branch");
    let decide_ix = fg
        .node_indices()
        .find(|&ix| fg[ix].last().map_or(false, |c| c.id == decide_id))
        .expect("the deciding branch must have a flow block");

    // The deciding branch's block must carry its full outputs - true when it is
    // the single-block body's terminal or the reconvergence join of inner
    // branches (pre-allocated against the full graph). Some multi-block shapes
    // (an inner branch reconverging *before* the deciding branch) can leave its
    // back-edge output unconnected in its block; reject those rather than
    // mis-compiling.
    let decide_outputs = fg[decide_ix].last().expect("non-empty block").conns.outputs;
    let back_outputs_present = loop_info
        .back_edges
        .iter()
        .filter(|(src, _)| *src == decide_id)
        .all(|(_, e)| decide_outputs.get(e.output.0 as usize).unwrap_or(false));
    if !back_outputs_present {
        return Err(CodegenError::UnsupportedLoopShape { header });
    }

    // Each loop-carried header input becomes a fn parameter, named after the
    // header's dedicated input binding so the header node-fn call uses it.
    let params: Vec<String> = loop_info
        .carried
        .iter()
        .map(|p| node_input_var(header, p.header_input))
        .collect();
    // Skip those inputs' dedicated-binding defines - they are the fn params.
    let mut body_phi = active_phi.clone();
    for p in &loop_info.carried {
        body_phi.insert((header, p.header_input));
    }

    // The loop fn body: recurse the ordinary lowering over the body with this
    // loop active. Inner forward branches reuse reconvergence/phi; the deciding
    // branch emits the loop decision and stops (see `flow_node_stmts`).
    let mut body_in_scope: HashSet<node::Id> = HashSet::new();
    let body = flow_node_stmts(
        path,
        flow_ix,
        mg,
        stateful,
        branching,
        inlets,
        outlets,
        reachable,
        fg,
        post_dom,
        phi_params,
        &mut body_in_scope,
        &body_phi,
        None,
        bridge_nodes,
        outlet_activity,
        Some(loop_info),
        loops,
    )?;

    // Emit the loop fn definition.
    let loop_fn = loop_fn_name(header);
    let body_str = body
        .iter()
        .map(|s| format!("{s}"))
        .collect::<Vec<_>>()
        .join(" ");
    let loop_fn_def = emit_one(&format!(
        "(define ({loop_fn} {}) {body_str})",
        params.join(" ")
    ));

    // The initial call - each param seeded from its external (pre-loop) source -
    // bound to the deciding branch's outputs var so downstream is unchanged.
    let branch_val = node_outputs_var(decide_id);
    let initial_args: Vec<String> = loop_info
        .carried
        .iter()
        .map(|p| match p.initial {
            Some((src, out)) => node_output_var(src, out.0 as usize),
            None => "'()".to_string(),
        })
        .collect();
    let call = emit_one(&format!(
        "(define {branch_val} ({loop_fn} {}))",
        initial_args.join(" ")
    ));

    let mut stmts = vec![loop_fn_def, call];
    in_scope.insert(decide_id);

    // Continue with the loop's downstream: the deciding branch's flow out-edges
    // are its exit arms. Destructure the result per arm and lower the successor
    // (outside the loop fn, so `current_loop` is the enclosing loop, if any).
    let mut edges: Vec<_> = fg
        .edges_directed(decide_ix, petgraph::Outgoing)
        .map(|e| (*e.weight(), e.target()))
        .collect();
    edges.sort();
    for (branch, dst) in edges {
        stmts.extend(destructure_node_outputs_stmt(decide_id, branch.conns));
        stmts.extend(flow_node_stmts(
            path,
            dst,
            mg,
            stateful,
            branching,
            inlets,
            outlets,
            reachable,
            fg,
            post_dom,
            phi_params,
            in_scope,
            active_phi,
            stop_at,
            bridge_nodes,
            outlet_activity,
            current_loop,
            loops,
        )?);
    }
    Ok(stmts)
}

/// Order flow-graph roots so that a root whose component produces a value
/// consumed by another root's component is emitted first.
///
/// `build_flow_graph` can leave a branch's reconvergence join in a different
/// flow component from a predecessor it depends on: the predecessor was lowered
/// on a separate root with no flow edge to the join (a branch resets the
/// topological chaining). The join is emitted inline with its branch, so its
/// cross-component predecessor must be emitted *before* that branch - otherwise
/// codegen references the predecessor's output before it is defined.
///
/// Components are stably topologically sorted by the cross-component data
/// dependencies in `mg`; the incoming (id-sorted) order breaks ties so output
/// stays deterministic.
fn order_roots(mg: &MetaGraph, fg: &FlowGraph, roots: Vec<NodeIndex>) -> Vec<NodeIndex> {
    if roots.len() < 2 {
        return roots;
    }
    // Forward-reachable node ids for each root (its flow component).
    let reach: Vec<HashSet<node::Id>> = roots
        .iter()
        .map(|&r| {
            let mut set = HashSet::new();
            let mut dfs = petgraph::visit::Dfs::new(fg, r);
            while let Some(b) = dfs.next(fg) {
                set.extend(fg[b].iter().map(|c| c.id));
            }
            set
        })
        .collect();

    // `preceded_by[i]` = roots that must be emitted before root `i`, because a
    // meta edge feeds from their (exclusive) reach into root `i`'s reach.
    let n = roots.len();
    let mut preceded_by: Vec<HashSet<usize>> = vec![HashSet::new(); n];
    for (u, v, _) in mg.all_edges() {
        for i in 0..n {
            if !reach[i].contains(&v) || reach[i].contains(&u) {
                continue;
            }
            for j in 0..n {
                if j != i && reach[j].contains(&u) {
                    preceded_by[i].insert(j);
                }
            }
        }
    }

    // Stable topological sort (Kahn): repeatedly emit the lowest-index root with
    // no remaining predecessor.
    let mut indegree: Vec<usize> = preceded_by.iter().map(|s| s.len()).collect();
    let mut emitted = vec![false; n];
    let mut order = Vec::with_capacity(n);
    while order.len() < n {
        let Some(i) = (0..n).find(|&i| !emitted[i] && indegree[i] == 0) else {
            // Unexpected cycle (the dataflow is acyclic); emit the rest in order.
            order.extend((0..n).filter(|&k| !emitted[k]).map(|k| roots[k]));
            break;
        };
        emitted[i] = true;
        order.push(roots[i]);
        for k in 0..n {
            if !emitted[k] && preceded_by[k].contains(&i) {
                indegree[k] -= 1;
            }
        }
    }
    order
}

/// Given the flow graph for an entry point eval fn, generate the body for the
/// fn. as a list of statements.
pub(crate) fn entry_fn_body(
    path: &[node::Id],
    mg: &MetaGraph,
    stateful: &BTreeSet<node::Id>,
    branching: &BTreeMap<node::Id, usize>,
    inlets: &BTreeSet<node::Id>,
    outlets: &BTreeSet<node::Id>,
    fg: &FlowGraph,
    bridge_nodes: &BTreeSet<node::Id>,
    outlet_activity: OutletActivity,
    loops: &LoopTable,
) -> Result<Vec<ExprKind>, CodegenError> {
    let roots = order_roots(mg, fg, flow_graph_roots(fg));
    if roots.is_empty() {
        return Ok(vec![]);
    }
    let reachable = flow_graph_nodes(fg);
    let join_points = find_join_points(fg);
    let post_dom = compute_post_dominators(fg);
    let mut phi_params_map = BTreeMap::new();
    for &j in &join_points {
        phi_params_map.insert(j, join_phi_params(mg, &reachable, &fg[j])?);
    }
    // A flow graph may contain multiple disconnected components (e.g. parallel
    // branches, or independent inlet→outlet chains). Generate each from its
    // root, sharing scope so bindings remain visible across the whole body.
    // Per-node `branch-ix-{id}` placeholders are declared up-front so each
    // branch can `set!` its selector and any arm can read it.
    let mut in_scope = HashSet::new();
    let mut stmts = declare_branch_ix_placeholders(branching);
    // A root graph (no enclosing graph node) may itself contain outlets - e.g.
    // a subgraph opened directly as a head. Nested levels have their outlet vars
    // hoisted by `wrap_outlet_bridge`; the root has no parent to do so, so hoist
    // them here. The values are never read at the root, so root outlets are
    // ignored.
    if path.is_empty() && !outlets.is_empty() {
        let decls = outlet_var_declarations(outlets, outlet_activity == OutletActivity::Tracked);
        for decl in decls {
            stmts.extend(Engine::emit_ast(&decl).expect("failed to emit root outlet declaration"));
        }
    }
    for root in roots {
        stmts.extend(flow_node_stmts(
            path,
            root,
            mg,
            stateful,
            branching,
            inlets,
            outlets,
            &reachable,
            fg,
            &post_dom,
            &phi_params_map,
            &mut in_scope,
            &HashSet::new(),
            None,
            bridge_nodes,
            outlet_activity,
            None,
            loops,
        )?);
    }
    Ok(stmts)
}

/// Wrap statements from a nested graph level with state scope enter/exit.
///
/// For a nested level at path `[graph_a]`, this generates:
/// ```scheme
/// (define __parent-graph-state graph-state)
/// (define graph-state (hash-ref __parent-graph-state '{graph_a_id}))
/// ;; ... nested statements ...
/// (set! graph-state (hash-insert __parent-graph-state '{graph_a_id} graph-state))
/// ```
fn wrap_state_scope(path: &[node::Id], stmts: Vec<ExprKind>) -> Vec<ExprKind> {
    use crate::GRAPH_STATE;

    if path.is_empty() || stmts.is_empty() {
        return stmts;
    }

    let mut result = Vec::with_capacity(stmts.len() + 2 * path.len());

    // Enter scopes from outermost to innermost.
    for (depth, &id) in path.iter().enumerate() {
        let parent = format!("__parent-graph-state-{depth}");
        let enter = format!(
            "(define {parent} {GRAPH_STATE}) \
             (set! {GRAPH_STATE} (hash-ref {parent} '{id}))"
        );
        result.extend(Engine::emit_ast(&enter).expect("failed to emit state scope enter"));
    }

    result.extend(stmts);

    // Exit scopes from innermost to outermost.
    for (depth, &id) in path.iter().enumerate().rev() {
        let parent = format!("__parent-graph-state-{depth}");
        let exit = format!("(set! {GRAPH_STATE} (hash-insert {parent} '{id} {GRAPH_STATE}))");
        result.extend(Engine::emit_ast(&exit).expect("failed to emit state scope exit"));
    }

    result
}

/// Wrap inner-level statements in a `let` block that returns the outlet values,
/// binding the result as the graph node's output in the parent scope.
///
/// The `let` block creates a new scope so inner node bindings (e.g. `node-0`)
/// don't collide with parent-level bindings. The tail expression - the graph
/// node's output - is the outlet values directly, or, when the push reaches the
/// outlets through branching (`patterns.len() >= 2`), a `(list branch-ix value)`
/// the parent branch-selects on (mirroring `node::graph::nested_expr`).
fn wrap_outlet_bridge(
    inner_stmts: Vec<ExprKind>,
    graph_node_id: node::Id,
    inner_outlets: &BTreeSet<node::Id>,
    reached_outlets: &BTreeSet<node::Id>,
    patterns: &[node::Conns],
) -> Vec<ExprKind> {
    if inner_stmts.is_empty() || reached_outlets.is_empty() {
        return inner_stmts;
    }

    let branching = patterns.len() >= 2;

    // Outlets write their values to hoisted `outlet-{id}` vars (see `eval_stmt`),
    // and - when branching - their `outlet-active-{id}` flags. Declare them
    // within the `let` so the inner statements set them and we read them back as
    // the graph node's output. Unreached outlets keep their `'()`/`#f` defaults.
    let declares = outlet_var_declarations(inner_outlets, branching).join(" ");

    let outlet_ids: Vec<node::Id> = inner_outlets.iter().copied().collect();
    let tail = if branching {
        branch_selector(patterns, &outlet_ids)
    } else {
        outlet_values_expr(&outlet_ids)
    };

    let inner_stmts_str = inner_stmts
        .iter()
        .map(|s| format!("{s}"))
        .collect::<Vec<_>>()
        .join(" ");

    let output_var = node_outputs_var(graph_node_id);
    let wrapped = format!("(define {output_var} (let () {declares} {inner_stmts_str} {tail}))");

    Engine::emit_ast(&wrapped).expect("failed to emit AST for outlet bridge")
}

/// Given a tree of eval plans for a gantz graph (and its nested graphs),
/// generate all entry fns.
///
/// Uses recursive post-order traversal so nested statements execute before
/// parent statements. When a nested entrypoint reaches outlets, its
/// statements are wrapped in a `let` block (outlet bridge) that returns
/// outlet values as the graph node's output, and parent-level continuation
/// statements are generated downstream.
pub(crate) fn entry_fns(
    flow_tree: &RoseTree<(&Meta, Flow)>,
) -> Result<Vec<ExprKind>, CodegenError> {
    let mut all_stmts: BTreeMap<super::EntrypointId, Vec<ExprKind>> = BTreeMap::new();
    entry_fns_collect(flow_tree, &[], &mut all_stmts)?;
    Ok(all_stmts
        .into_iter()
        .map(|(id, stmts)| entry_fn(&entry_fn_name(&id), stmts))
        .collect())
}

/// Recursively collect entry fn statements from the flow tree.
///
/// Post-order: recurse into children first. When a child entrypoint reaches
/// outlets, wrap its statements in an outlet bridge and generate parent-level
/// continuation statements downstream of the graph node.
fn entry_fns_collect(
    tree: &RoseTree<(&Meta, Flow)>,
    path: &[node::Id],
    all_stmts: &mut BTreeMap<super::EntrypointId, Vec<ExprKind>>,
) -> Result<(), CodegenError> {
    // Track which entrypoints have outlet bridges at this level, keyed by
    // (EntrypointId, graph_node_id) -> bridged stmts.
    let mut bridged: BTreeMap<super::EntrypointId, Vec<ExprKind>> = BTreeMap::new();
    let mut bridge_node_ids: BTreeMap<super::EntrypointId, BTreeSet<node::Id>> = BTreeMap::new();
    // Bridge graph nodes whose push reaches outlets through branching, so the
    // parent treats them as branch nodes for that entrypoint: node -> arm count.
    let mut branch_bridges: BTreeMap<super::EntrypointId, BTreeMap<node::Id, usize>> =
        BTreeMap::new();

    // 1. Recurse into children (post-order).
    for (&graph_node_id, child_tree) in &tree.nested {
        let mut child_path = path.to_vec();
        child_path.push(graph_node_id);

        // Collect child stmts into a temporary map so we can intercept
        // outlet-reaching entrypoints before they go into all_stmts.
        let mut child_stmts: BTreeMap<super::EntrypointId, Vec<ExprKind>> = BTreeMap::new();
        entry_fns_collect(child_tree, &child_path, &mut child_stmts)?;

        let (child_meta, ref child_flow) = child_tree.elem;

        for (ep_id, stmts) in child_stmts {
            if let Some(reach) = child_flow.outlet_reach.get(&ep_id) {
                // Child stmts already include state scope wrapping from the
                // child's own entry_fns_collect. Wrap in an outlet bridge that
                // binds `node-{graph_node}`; when the child's push reaches its
                // outlets through branching, the bridge yields a
                // `(list branch-ix value)` the parent branch-selects on.
                let bridge = wrap_outlet_bridge(
                    stmts,
                    graph_node_id,
                    &child_meta.outlets,
                    &reach.reached,
                    &reach.patterns,
                );
                bridged.entry(ep_id.clone()).or_default().extend(bridge);
                bridge_node_ids
                    .entry(ep_id.clone())
                    .or_default()
                    .insert(graph_node_id);
                if reach.patterns.len() >= 2 {
                    branch_bridges
                        .entry(ep_id)
                        .or_default()
                        .insert(graph_node_id, reach.patterns.len());
                }
            } else {
                // No outlet propagation. Child stmts already include state
                // scope wrapping from the child's own entry_fns_collect.
                all_stmts.entry(ep_id).or_default().extend(stmts);
            }
        }
    }

    // 2. Generate this level's own entrypoint stmts.
    let (meta, flow) = &tree.elem;
    let graph_branching: BTreeMap<node::Id, usize> =
        meta.branches.iter().map(|(&id, v)| (id, v.len())).collect();

    // Determine bridge_nodes for each entrypoint (graph nodes whose output
    // is already bound by an outlet bridge).
    let empty_bridges = BTreeSet::new();
    let empty_branch_bridges = BTreeMap::new();
    let empty_loops = LoopTable::new();

    for (ep_id, fg) in &flow.entrypoints {
        let bn = bridge_node_ids.get(ep_id).unwrap_or(&empty_bridges);
        // Per-entrypoint branch nodes: this graph's own branches plus any
        // branch-aware push-through bridges for this entrypoint.
        let mut branching = graph_branching.clone();
        branching.extend(
            branch_bridges
                .get(ep_id)
                .unwrap_or(&empty_branch_bridges)
                .iter()
                .map(|(&id, &n)| (id, n)),
        );
        // Track outlet activation iff this graph's own push reaches its outlets
        // through branching, so its parent can branch-select on the flags
        // (mirrors `node::graph::nested_expr`).
        let outlet_activity = if flow
            .outlet_reach
            .get(ep_id)
            .map_or(false, |reach| reach.patterns.len() >= 2)
        {
            OutletActivity::Tracked
        } else {
            OutletActivity::Untracked
        };
        let stmts = entry_fn_body(
            path,
            &meta.graph,
            &meta.stateful,
            &branching,
            &meta.inlets,
            &meta.outlets,
            fg,
            bn,
            outlet_activity,
            flow.loops.get(ep_id).unwrap_or(&empty_loops),
        )?;
        let scoped = wrap_state_scope(path, stmts);

        // If there are bridged stmts for this entrypoint, prepend them.
        if let Some(bridge_stmts) = bridged.remove(ep_id) {
            let entry = all_stmts.entry(ep_id.clone()).or_default();
            entry.extend(bridge_stmts);
            entry.extend(scoped);
        } else {
            all_stmts.entry(ep_id.clone()).or_default().extend(scoped);
        }
    }

    // 3. Handle any remaining bridged entrypoints that have no parent-level
    //    continuation (e.g. parent has no downstream from the graph node).
    for (ep_id, bridge_stmts) in bridged {
        all_stmts.entry(ep_id).or_default().extend(bridge_stmts);
    }

    Ok(())
}
