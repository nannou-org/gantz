//! Items related to lowering the control `Flow` graph to steel code.

use crate::{
    GRAPH_STATE, ROOT_STATE,
    compile::{
        Block, Flow, FlowGraph, Meta, MetaGraph, NodeConns, RoseTree,
        error::{CodegenError, InvalidInputIndex, TooManyConns},
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
/// Binding used for the current branch index.
const BRANCH_IX: &str = "branch-ix";

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

    // Skip outlet nodes entirely - their values are read directly from source
    // node output bindings by nested_expr.
    if outlets.contains(&node_ix) {
        return Ok(None);
    }

    // Determine the input arg names.
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
fn phi_set_stmts(
    mg: &MetaGraph,
    pred_last_node: node::Id,
    join_node_id: node::Id,
    phi_params: &[(usize, String)],
) -> Vec<ExprKind> {
    // Collect all edges from the predecessor to the join node.
    let edges: Vec<_> = mg
        .edges_directed(join_node_id, petgraph::Incoming)
        .filter(|e| e.source() == pred_last_node)
        .flat_map(|e| e.weight().clone())
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
) -> Result<Vec<ExprKind>, CodegenError> {
    let mut stmts = Vec::new();
    let mut iter = block.0.iter().peekable();
    while let Some(conf) = iter.next() {
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
            mg, reachable, inlets, outlets, &node_path, &inputs, &outputs, stateful,
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
    inlets: &BTreeSet<node::Id>,
    outlets: &BTreeSet<node::Id>,
    reachable: &HashSet<node::Id>,
    fg: &FlowGraph,
    post_dom: &HashMap<NodeIndex, NodeIndex>,
    phi_params: &BTreeMap<NodeIndex, Vec<(usize, String)>>,
    in_scope: &mut HashSet<node::Id>,
    active_phi: &HashSet<(node::Id, usize)>,
    stop_at: Option<NodeIndex>,
) -> Result<Vec<ExprKind>, CodegenError> {
    // Add the block.
    let block = &fg[flow_ix];
    let mut stmts = eval_fn_block_stmts(
        path, mg, stateful, inlets, outlets, reachable, block, in_scope, active_phi,
    )?;

    // Collect the output branching edges.
    let mut edges: Vec<_> = fg
        .edges_directed(flow_ix, petgraph::Outgoing)
        .map(|e_ref| (*e_ref.weight(), e_ref.target()))
        .collect();
    edges.sort();

    // If there are no edges, we're done.
    if edges.is_empty() {
        return Ok(stmts);

    // Single successor: either a linear continuation or leads to a join.
    } else if edges.len() == 1 {
        let (_branch, dst) = edges.pop().unwrap();
        let conf = *block.last().unwrap();
        stmts.extend(destructure_node_outputs_stmt(conf.id, conf.conns.outputs));
        in_scope.insert(conf.id);

        // If the successor is the reconvergence point we're stopping at,
        // emit phi set statements and return.
        if stop_at == Some(dst) {
            let join_first_id = fg[dst].first().expect("join block must not be empty").id;
            if let Some(params) = phi_params.get(&dst) {
                stmts.extend(phi_set_stmts(mg, conf.id, join_first_id, params));
            }
            return Ok(stmts);
        }

        // Otherwise continue normally. The next block's
        // `eval_fn_block_stmts` handles input bindings via before-target.
        stmts.extend(flow_node_stmts(
            path, dst, mg, stateful, inlets, outlets, reachable, fg, post_dom, phi_params,
            in_scope, active_phi, stop_at,
        )?);
        return Ok(stmts);
    }

    // Multiple edges: this is a branch.
    let conf = *block.last().unwrap();
    stmts.push(destructure_node_branch_stmt(conf.id));

    // Post-dominator gives the reconvergence point directly.
    // Skip if it equals stop_at (handled by outer level).
    let reconvergence = post_dom
        .get(&flow_ix)
        .copied()
        .filter(|&r| stop_at != Some(r));

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
                    branch_stmts.extend(phi_set_stmts(mg, conf.id, join_first_id, params));
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
                    inlets,
                    outlets,
                    reachable,
                    fg,
                    post_dom,
                    phi_params,
                    &mut arm_scope,
                    &inner_phi,
                    Some(join_ix),
                )?);
            }

            let branch_str = branch_stmts
                .iter()
                .map(|s| format!("{s}"))
                .collect::<Vec<_>>()
                .join(" ");
            expr = format!(
                "(if (= {} {BRANCH_IX}) (begin {branch_str}) {expr})",
                branch.ix
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
            path, join_ix, mg, stateful, inlets, outlets, reachable, fg, post_dom, phi_params,
            in_scope, &inner_phi, stop_at,
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
            inlets,
            outlets,
            reachable,
            fg,
            post_dom,
            phi_params,
            &mut arm_scope,
            active_phi,
            stop_at,
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
        expr = format!("(if (= {} {BRANCH_IX}) {dst_expr} {expr})", branch.ix);
    }

    let expr = Engine::emit_ast(&expr)
        .expect("failed to emit AST for branch")
        .into_iter()
        .next()
        .unwrap();

    stmts.push(expr);
    Ok(stmts)
}

/// Given the flow graph for an entry point eval fn, generate the body for the
/// fn. as a list of statements.
pub fn entry_fn_body(
    path: &[node::Id],
    mg: &MetaGraph,
    stateful: &BTreeSet<node::Id>,
    inlets: &BTreeSet<node::Id>,
    outlets: &BTreeSet<node::Id>,
    fg: &FlowGraph,
) -> Result<Vec<ExprKind>, CodegenError> {
    let Some(entry) = flow_graph_entry(fg) else {
        return Ok(vec![]);
    };
    let reachable = flow_graph_nodes(fg);
    let join_points = find_join_points(fg);
    let post_dom = compute_post_dominators(fg);
    let mut phi_params_map = BTreeMap::new();
    for &j in &join_points {
        phi_params_map.insert(j, join_phi_params(mg, &reachable, &fg[j])?);
    }
    flow_node_stmts(
        path,
        entry.id(),
        mg,
        stateful,
        inlets,
        outlets,
        &reachable,
        fg,
        &post_dom,
        &phi_params_map,
        &mut HashSet::new(),
        &HashSet::new(),
        None,
    )
}

/// Generate eval statements for each entrypoint at this graph level.
///
/// Returns a map from `EntrypointId` to the statements for that entrypoint
/// at this level. Cross-level entrypoints will have statements at multiple
/// levels, which are collected and concatenated by `entry_fns`.
pub(crate) fn eval_stmts_from_flow(
    path: &[node::Id],
    mg: &MetaGraph,
    stateful: &BTreeSet<node::Id>,
    inlets: &BTreeSet<node::Id>,
    outlets: &BTreeSet<node::Id>,
    flow: &Flow,
) -> Result<BTreeMap<super::EntrypointId, Vec<ExprKind>>, CodegenError> {
    flow.entrypoints
        .iter()
        .map(|(id, fg)| {
            let stmts = entry_fn_body(path, mg, stateful, inlets, outlets, fg)?;
            Ok((id.clone(), stmts))
        })
        .collect()
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

/// Given a tree of eval plans for a gantz graph (and its nested graphs),
/// generate all entry fns.
///
/// Uses post-order traversal so nested statements execute before parent
/// statements. Nested-level statements are wrapped with state scope
/// enter/exit to narrow `graph-state` to the correct sub-hashmap.
pub(crate) fn entry_fns(
    flow_tree: &RoseTree<(&Meta, Flow)>,
) -> Result<Vec<ExprKind>, CodegenError> {
    // Collect statements from all levels, grouped by EntrypointId.
    // Post-order: children before parent, so nested eval runs first.
    let mut all_stmts: BTreeMap<super::EntrypointId, Vec<ExprKind>> = BTreeMap::new();
    flow_tree.try_visit_post(&[], &mut |path, (meta, flow)| -> Result<(), CodegenError> {
        let level_stmts = eval_stmts_from_flow(
            path,
            &meta.graph,
            &meta.stateful,
            &meta.inlets,
            &meta.outlets,
            flow,
        )?;
        for (id, stmts) in level_stmts {
            let scoped = wrap_state_scope(path, stmts);
            all_stmts.entry(id).or_default().extend(scoped);
        }
        Ok(())
    })?;

    // Generate one eval fn per EntrypointId.
    Ok(all_stmts
        .into_iter()
        .map(|(id, stmts)| entry_fn(&entry_fn_name(&id), stmts))
        .collect())
}
