use crate::{
    Edge, GRAPH_STATE, ROOT_STATE,
    node::{self, Node},
};
use petgraph::visit::{
    Data, Dfs, EdgeRef, GraphRef, IntoEdgesDirected, IntoNodeReferences, NodeIndexable, NodeRef,
    Topo, Visitable, Walker,
};
use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
};
use steel::{parser::ast::ExprKind, steel_vm::engine::Engine};

/// An evaluation step ready for translation to code.
#[derive(Debug)]
pub struct EvalStep<NI> {
    /// The node to be evaluated.
    pub node: NI,
    /// Arguments to the node's function call.
    ///
    /// The `len` of the outer vec will always be equal to the number of inputs
    /// on `node`.
    pub args: Vec<Option<ExprInput<NI>>>,
}

/// An argument to a node's function call.
#[derive(Debug)]
pub struct ExprInput<NI> {
    /// The node from which the value was generated.
    pub node: NI,
    /// The output on the source node associated with the generated value.
    pub output: node::Output,
    /// Whether or not using the value in this argument requires cloning.
    pub requires_clone: bool,
}

// An expression for a node's key into the graph state hashmap.
pub fn node_state_key(node_id: usize) -> ExprKind {
    // Create a symbol or other hashable key to use in the hashmap
    let key_str = format!("'{node_id}");
    Engine::emit_ast(&key_str)
        .expect("failed to emit AST")
        .into_iter()
        .next()
        .unwrap()
}

/// Given a graph of gantz nodes, return `NodeId`s of those that require push
/// evaluation.
///
/// Expects any graph type whose nodes implement `Node`.
pub fn push_nodes<G>(g: G) -> Vec<(G::NodeId, String)>
where
    G: IntoNodeReferences + NodeIndexable,
    G::NodeWeight: Node,
{
    g.node_references()
        .filter_map(|n| {
            let id = n.id();
            let ix = g.to_index(id);
            let name = push_eval_fn_name(ix);
            n.weight().push_eval().map(|_eval| (id, name))
        })
        .collect()
}

/// Given a graph of gantz nodes, return `NodeId`s of those that require pull
/// evaluation.
///
/// Expects any graph type whose nodes implement `Node`.
pub fn pull_nodes<G>(g: G) -> Vec<(G::NodeId, String)>
where
    G: IntoNodeReferences + NodeIndexable,
    G::NodeWeight: Node,
{
    g.node_references()
        .filter_map(|n| {
            let id = n.id();
            let ix = g.to_index(id);
            let name = pull_eval_fn_name(ix);
            n.weight().pull_eval().map(|_eval| (id, name))
        })
        .collect()
}

/// An iterator yielding all nodes reachable via pushing from the given node.
pub fn push_reachable<G>(g: G, n: G::NodeId) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + Visitable,
{
    Dfs::new(g, n).iter(g)
}

/// An iterator yielding all nodes reachable via pulling from the given node.
pub fn pull_reachable<G>(g: G, n: G::NodeId) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + Visitable,
{
    let rev_g = petgraph::visit::Reversed(g);
    Dfs::new(rev_g, n).iter(rev_g)
}

/// Push evaluation from the specified node.
///
/// Evaluation order is equivalent to a topological ordering of the connected
/// component starting from the given node.
///
/// Expects any directed graph whose edges are of type `Edge` and whose nodes
/// implement `Node`. Direction of edges indicate the flow of data through the
/// graph.
pub fn push_eval_order<G>(g: G, n: G::NodeId) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + IntoNodeReferences + Visitable,
    G::NodeId: Eq + Hash,
{
    let dfs: HashSet<G::NodeId> = push_reachable(g, n).collect();
    Topo::new(g).iter(g).filter(move |node| dfs.contains(&node))
}

/// Pull evaluation from the specified node.
///
/// Evaluation order is equivalent to a topological ordering of the connected
/// component that ends at the given node.
///
/// Expects any directed graph whose edges are of type `Edge` and whose nodes
/// implement `Node`. Direction of edges indicate the flow of data through the
/// graph.
pub fn pull_eval_order<G>(g: G, n: G::NodeId) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + IntoNodeReferences + Visitable,
    G::NodeId: Eq + Hash,
{
    let dfs: HashSet<G::NodeId> = pull_reachable(g, n).collect();
    Topo::new(g).iter(g).filter(move |node| dfs.contains(&node))
}

/// The evaluation order for given any number of simultaneously pushing and
/// pulling nodes.
///
/// Evaluation order is equivalent to a topological ordering of the connected
/// components reachable via DFS from each push node and reversed-edge DFS from
/// each pull node.
///
/// Expects any directed graph whose edges are of type `Edge` and whose nodes
/// implement `Node`. Direction of edges indicate the flow of data through the
/// graph.
pub fn eval_order<G, A, B>(g: G, push: A, pull: B) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + IntoNodeReferences + Visitable,
    G::NodeId: Eq + Hash,
    A: IntoIterator<Item = G::NodeId>,
    B: IntoIterator<Item = G::NodeId>,
{
    let mut reachable = HashSet::new();
    reachable.extend(push.into_iter().flat_map(|n| push_reachable(g, n)));
    reachable.extend(pull.into_iter().flat_map(|n| pull_reachable(g, n)));
    Topo::new(g).iter(g).filter(move |n| reachable.contains(&n))
}

/// Given a node evaluation order, produce the series of evaluation steps
/// required.
pub fn eval_steps<G, I>(g: G, eval_order: I) -> Vec<EvalStep<G::NodeId>>
where
    G: IntoEdgesDirected + IntoNodeReferences + NodeIndexable,
    G: Data<EdgeWeight = Edge>,
    G::NodeId: Eq + Hash,
    G::NodeWeight: Node,
    I: IntoIterator<Item = G::NodeId>,
{
    let mut eval_steps = vec![];
    let mut visited = HashSet::new();

    // Step through each of the nodes.
    for node in eval_order {
        visited.insert(node);

        // Initialise the arguments to `None` for each input.
        // FIXME: Use some node-indexing trait instead.
        let n = g.node_references().find(|n| n.id() == node).unwrap();
        let n_inputs = n.weight().n_inputs();
        let mut args: Vec<_> = (0..n_inputs).map(|_| None).collect();

        // Create an argument for each input to this child.
        for e_ref in g.edges_directed(node, petgraph::Incoming) {
            // Only consider edges to nodes that we have already visited.
            if !visited.contains(&e_ref.source()) {
                continue;
            }

            let w = e_ref.weight();

            // Check how many connections their are from the parent's output and
            // see if the value will need to be cloned when passed to this input.
            let requires_clone = {
                let parent = e_ref.source();
                // TODO: Connection order should match
                let mut connection_ix = 0;
                let mut total_connections_from_output = 0;
                for (i, pe_ref) in g.edges_directed(parent, petgraph::Outgoing).enumerate() {
                    let pw = pe_ref.weight();
                    if pw == w {
                        connection_ix = i;
                    }
                    if pw.output == w.output {
                        total_connections_from_output += 1;
                    }
                }
                total_connections_from_output > 1
                    && connection_ix < (total_connections_from_output - 1)
            };

            // Assign the expression argument for this input.
            let arg = ExprInput {
                node: e_ref.source(),
                output: w.output,
                requires_clone,
            };
            args[w.input.0 as usize] = Some(arg);
        }

        // Add the step.
        eval_steps.push(EvalStep { node, args });
    }

    eval_steps
}

/// Generate a sequence of evaluation statements, one for each given evaluation
/// step.
pub fn eval_stmts<G>(g: G, steps: &[EvalStep<G::NodeId>]) -> Vec<ExprKind>
where
    G: Data + GraphRef + IntoNodeReferences + NodeIndexable,
    G::NodeId: Eq + Hash,
    G::NodeWeight: Node,
{
    type OutputVars<NI> = HashMap<(NI, node::Output), String>;

    // Track output variables
    let mut output_vars: OutputVars<G::NodeId> = HashMap::new();
    let mut stmts = Vec::new();

    // Function to generate variable names
    fn var_name(node_ix: usize, out_ix: u16) -> String {
        format!("__node{}_output{}", node_ix, out_ix)
    }

    // Update wrap_node_expr_with_state to extract state from the hashmap
    fn wrap_node_expr_with_state(node_expr: &ExprKind, key: &ExprKind) -> ExprKind {
        // 1. Gets the current state
        // 2. Evaluate the node expr (which may modify the local state var)
        // 3. Updates the hashmap with the potentially modified state
        // 4. Returns the result of the node expression
        let expr_str = format!(
            "(let ((state (hash-ref {GRAPH_STATE} {key})))
               (let ((result {node_expr}))
                 (set! {GRAPH_STATE} (hash-insert {GRAPH_STATE} {key} state))
                 result))"
        );

        Engine::emit_ast(&expr_str)
            .expect("failed to emit AST")
            .into_iter()
            .next()
            .unwrap()
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

    // Create an expression that references a variable name
    fn create_var_expr(var_name: &str) -> ExprKind {
        Engine::emit_ast(var_name)
            .expect("failed to emit AST")
            .into_iter()
            .next()
            .unwrap()
    }

    for (step_ix, step) in steps.iter().enumerate() {
        // FIXME: Use some node indexing trait.
        let node = g.node_references().find(|n| n.id() == step.node).unwrap();
        let n_outputs = node.weight().n_outputs();

        // Create variables for this node's outputs
        for out_ix in 0..n_outputs {
            let var = var_name(step_ix, out_ix as u16);
            output_vars.insert((step.node, node::Output(out_ix as u16)), var);
        }

        // Prepare input expressions for the node
        let input_exprs: Vec<_> = step
            .args
            .iter()
            .map(|arg_opt| {
                arg_opt.as_ref().map(|arg| {
                    // Get the variable name for this input
                    let var_name = output_vars.get(&(arg.node, arg.output)).unwrap();
                    // Create an expression referencing this variable
                    if arg.requires_clone {
                        // TODO: Is cloning of objects implicit in steel?
                        // create_clone_expr(var_name)
                        create_var_expr(var_name)
                    } else {
                        create_var_expr(var_name)
                    }
                })
            })
            .collect();

        // Work out the node's state variable name.
        let node_ix = g.to_index(step.node);
        let node_state_key = node_state_key(node_ix);

        // Get the node's expression.
        let mut node_expr = node.weight().expr(&input_exprs);
        if node.weight().stateful() {
            node_expr = wrap_node_expr_with_state(&node_expr, &node_state_key);
        }

        // Create a binding statement for each output
        match n_outputs {
            0 => stmts.push(node_expr),
            1 => {
                let output_var = var_name(step_ix, 0);
                let define_expr = create_define_expr(output_var, node_expr);
                stmts.push(define_expr);
            }
            _ => {
                let output_vars: Vec<String> = (0..n_outputs)
                    .map(|i| var_name(step_ix, i as u16))
                    .collect();
                let define_values_expr = create_define_values_expr(output_vars, node_expr);
                stmts.push(define_values_expr);
            }
        }
    }

    stmts
}

/// The name used for the pull evaluation function generated for the given node.
pub fn pull_eval_fn_name(id: node::Id) -> String {
    format!("pull_eval_{id}")
}

/// The name used for the push evaluation function generated for the given node.
pub fn push_eval_fn_name(id: node::Id) -> String {
    format!("push_eval_{id}")
}

/// Generate a function for performing evaluation of the given statements.
///
/// The given `Vec<ExprKind>` should be generated via the `eval_stmts` function.
pub fn eval_fn(eval_fn_name: &str, stmts: Vec<ExprKind>) -> ExprKind {
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

/// Given a list of push evaluation nodes and their evaluation steps, generate a
/// function for performing push evaluation for each node.
pub fn eval_fns<'a, G, I>(g: G, eval_nodes: I) -> Vec<ExprKind>
where
    G: GraphRef + IntoNodeReferences + NodeIndexable,
    G::NodeId: 'a + Eq + Hash,
    G::NodeWeight: Node,
    I: IntoIterator<Item = (G::NodeId, String, &'a [EvalStep<G::NodeId>])>,
{
    eval_nodes
        .into_iter()
        .map(|(_n, eval_fn_name, steps)| {
            let stmts = eval_stmts(g, steps);
            eval_fn(&eval_fn_name, stmts)
        })
        .collect()
}

/// Given a graph, generate the full module with all the necessary functions for
/// executing it.
pub fn module<G>(g: G, inlets: &[G::NodeId], outlets: &[G::NodeId]) -> Vec<ExprKind>
where
    G: GraphRef + IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable,
    G: Data<EdgeWeight = Edge>,
    G::NodeId: Eq + Hash,
    G::NodeWeight: Node,
{
    let full_eval_steps = match (inlets.is_empty(), outlets.is_empty()) {
        (true, true) => None,
        _ => {
            /// The name of the function generated for performing full
            /// evaluation of the graph.
            const FULL_EVAL_FN_NAME: &str = "full_eval";
            let eval_fn_name = FULL_EVAL_FN_NAME.to_string();
            let order = eval_order(g, inlets.iter().cloned(), outlets.iter().cloned());
            let steps = eval_steps(g, order);
            Some((steps, eval_fn_name))
        }
    };

    let pull_nodes = pull_nodes(g);
    let push_nodes = push_nodes(g);
    let pull_node_eval_steps = pull_nodes.into_iter().map(|(n, eval)| {
        let order = pull_eval_order(g, n);
        let steps = eval_steps(g, order);
        (steps, eval)
    });
    let push_node_eval_steps = push_nodes.into_iter().map(|(n, eval)| {
        let order = push_eval_order(g, n);
        let steps = eval_steps(g, order);
        (steps, eval)
    });

    full_eval_steps
        .into_iter()
        .chain(pull_node_eval_steps)
        .chain(push_node_eval_steps)
        .map(|(steps, eval_fn_name)| {
            let stmts = eval_stmts(g, &steps);
            eval_fn(&eval_fn_name, stmts)
        })
        .collect()
}
