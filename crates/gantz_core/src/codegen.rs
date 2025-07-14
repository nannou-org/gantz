//! Items related to generating steel code from a gantz graph, primarily the
//! [`module`] fn.

use crate::{
    Edge, GRAPH_STATE, ROOT_STATE,
    node::{self, Node},
    visit::{self, Visitor},
};
#[doc(inline)]
pub use flow::Flow;
use petgraph::visit::{
    Data, Dfs, EdgeRef, IntoEdgesDirected, IntoNodeReferences, NodeIndexable, Topo, Visitable,
    Walker,
};
pub(crate) use rosetree::RoseTree;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    hash::Hash,
};
use steel::{parser::ast::ExprKind, steel_vm::engine::Engine};

mod flow;
mod rosetree;

/// A representation of how to evaluate a graph.
///
/// Produced via [`eval_plan`].
#[derive(Debug)]
struct EvalPlan<'a> {
    /// The gantz graph `Flow` from which this `EvalPlan` was produced.
    flow: &'a Flow,
    /// Order of evaluation from all inlets to all outlets.
    // TODO: Knowing the connectedness of the inlets/outlets would be useful
    // for generating only the necessary node configs.
    // Empty in the case that the graph has no inlets or outlets (i.e. is not
    // nested).
    nested_steps: Vec<EvalStep>,
    /// The order of node evaluation for each push_eval node.
    push_steps: BTreeMap<node::Id, Vec<EvalStep>>,
    /// The order of node evaluation for each pull_eval node.
    pull_steps: BTreeMap<node::Id, Vec<EvalStep>>,
}

/// An evaluation step ready for translation to code.
///
/// Represents evaluation of a node with some set of the inputs connected.
#[derive(Debug)]
pub(crate) struct EvalStep {
    /// The node to be evaluated.
    pub(crate) node: node::Id,
    /// Arguments to the node's function call.
    ///
    /// The `len` of the outer vec will always be equal to the number of inputs
    /// on `node`.
    pub(crate) args: Vec<Option<ExprInput>>,
}

/// An argument to a node's function call.
#[derive(Debug)]
pub(crate) struct ExprInput {
    /// The node from which the value was generated.
    pub(crate) node: node::Id,
    /// The output on the source node associated with the generated value.
    pub(crate) output: node::Output,
}

/// The set of all node input configurations for a single graph.
///
/// These are used to determine which set of functions to generate for each
/// node.
type NodeConfs = BTreeSet<(node::Id, Vec<bool>)>;

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
        use std::ops::Bound::Included;
        let node_path = ctx.path();
        let plan_path = &node_path[..node_path.len() - 1];
        let tree = self.tree.tree(&plan_path).unwrap();
        let id = ctx.id();
        let n_inputs = node.n_inputs();
        let start = (id, vec![]);
        let end = (id, vec![true; n_inputs]);
        let range = (Included(start), Included(end));
        let input_confs = tree.elem.range(range);
        for (_id, conf) in input_confs {
            self.fns.push(node_fn(node, node_path, &conf));
        }
    }
}

/// The name used for the pull evaluation fn generated for the given node.
pub fn pull_eval_fn_name(path: &[node::Id]) -> String {
    format!("pull_eval_{}", path_string(path))
}

/// The name used for the push evaluation fn generated for the given node.
pub fn push_eval_fn_name(path: &[node::Id]) -> String {
    format!("push_eval_{}", path_string(path))
}

/// An iterator yielding all nodes reachable via pushing from the given node.
fn push_reachable<G>(g: G, n: G::NodeId) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + Visitable,
{
    Dfs::new(g, n).iter(g)
}

/// An iterator yielding all nodes reachable via pulling from the given node.
fn pull_reachable<G>(g: G, n: G::NodeId) -> impl Iterator<Item = G::NodeId>
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
fn push_eval_order<G>(g: G, n: G::NodeId) -> impl Iterator<Item = G::NodeId>
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
fn pull_eval_order<G>(g: G, n: G::NodeId) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + IntoNodeReferences + Visitable,
    G::NodeId: Eq + Hash,
{
    let dfs: HashSet<G::NodeId> = pull_reachable(g, n).collect();
    Topo::new(g).iter(g).filter(move |node| dfs.contains(&node))
}

/// The evaluation order given any number of simultaneously pushing and pulling
/// nodes.
///
/// Evaluation order is equivalent to a topological ordering of the connected
/// components reachable via DFS from each push node and reversed-edge DFS from
/// each pull node.
///
/// Expects any directed graph whose edges are of type `Edge` and whose nodes
/// implement `Node`. Direction of edges indicate the flow of data through the
/// graph.
pub(crate) fn eval_order<G, A, B>(g: G, push: A, pull: B) -> impl Iterator<Item = G::NodeId>
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
pub(crate) fn eval_steps<I>(flow: &Flow, eval_order: I) -> impl Iterator<Item = EvalStep>
where
    I: IntoIterator<Item = node::Id>,
{
    // Step through each of the nodes.
    let mut visited = HashSet::new();
    eval_order.into_iter().map(move |n| {
        visited.insert(n);

        // Initialise the arguments to `None` for each input.
        let n_inputs = flow.inputs.get(&n).copied().unwrap_or(0);
        let mut args: Vec<_> = (0..n_inputs).map(|_| None).collect();

        // Create an argument for each input to this child.
        for e_ref in flow.graph.edges_directed(n, petgraph::Incoming) {
            // Only consider edges to nodes that we have already visited.
            if !visited.contains(&e_ref.source()) {
                continue;
            }
            for edge in e_ref.weight() {
                // Assign the expression argument for this input.
                let arg = ExprInput {
                    node: e_ref.source(),
                    output: edge.output,
                };
                args[edge.input.0 as usize] = Some(arg);
            }
        }
        EvalStep { node: n, args }
    })
}

/// Create the evaluation plan for the graph associated with the given flow.
fn eval_plan(flow: &Flow) -> EvalPlan {
    let pull_steps = flow
        .pull
        .iter()
        .map(|&n| {
            let order = pull_eval_order(&flow.graph, n);
            let steps = eval_steps(flow, order).collect();
            (n, steps)
        })
        .collect();

    let push_steps = flow
        .push
        .iter()
        .map(|&n| {
            let order = push_eval_order(&flow.graph, n);
            let steps = eval_steps(flow, order).collect();
            (n, steps)
        })
        .collect();

    let nested_steps = {
        let order = eval_order(
            &flow.graph,
            flow.inlets.iter().cloned(),
            flow.outlets.iter().cloned(),
        );
        eval_steps(flow, order).collect()
    };

    EvalPlan {
        flow,
        push_steps,
        pull_steps,
        nested_steps,
    }
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

/// Generate a sequence of evaluation statements for an eval function, one
/// statement for each given evaluation step.
///
/// The given `path`, `outputs` and `stateful` are associated with the graph
/// in which the eval fn is invoked.
pub(crate) fn eval_stmts(
    path: &[node::Id],
    steps: &[EvalStep],
    outputs: &BTreeMap<node::Id, usize>,
    stateful: &BTreeSet<node::Id>,
) -> Vec<ExprKind> {
    type OutputVars = HashMap<(node::Id, node::Output), String>;

    const STATE: &str = "state";

    // Track output variables
    let mut output_vars: OutputVars = HashMap::new();
    let mut stmts = Vec::new();

    // Function to generate variable names
    fn var_name(node_ix: usize, out_ix: u16) -> String {
        format!("__node{}_output{}", node_ix, out_ix)
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

    // Create an expression that references a variable name
    fn create_var_expr(var_name: &str) -> ExprKind {
        Engine::emit_ast(var_name)
            .expect("failed to emit AST")
            .into_iter()
            .next()
            .unwrap()
    }

    for (step_ix, step) in steps.iter().enumerate() {
        let n_outputs = outputs.get(&step.node).copied().unwrap_or(0);

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
                    create_var_expr(var_name)
                })
            })
            .collect();

        // Get the node's function name.
        let node_path: Vec<_> = path.iter().copied().chain(Some(step.node)).collect();
        let node_inputs: Vec<_> = step.args.iter().map(|arg| arg.is_some()).collect();
        let node_fn_name = node_fn_name(&node_path, &node_inputs);

        // Prepare function arguments.
        let mut args: Vec<String> = input_exprs
            .iter()
            .filter_map(|opt| opt.as_ref().map(|expr| format!("{expr}")))
            .collect();
        let is_stateful = stateful.contains(&step.node);
        if is_stateful {
            args.push(STATE.to_string());
        }

        // The expression for the node function call.
        let mut node_fn_call_expr_str = format!("({node_fn_name} {})", args.join(" "));

        // Create the expression for the node.
        if is_stateful {
            node_fn_call_expr_str = wrap_node_fn_call_with_state(&node_fn_call_expr_str, step.node);
        };
        let node_fn_call_expr = Engine::emit_ast(&node_fn_call_expr_str)
            .expect("failed to emit AST")
            .into_iter()
            .next()
            .unwrap();

        // Create a binding statement for each output
        match n_outputs {
            0 => stmts.push(node_fn_call_expr),
            1 => {
                let output_var = var_name(step_ix, 0);
                let define_expr = create_define_expr(output_var, node_fn_call_expr);
                stmts.push(define_expr);
            }
            _ => {
                let output_vars: Vec<String> = (0..n_outputs)
                    .map(|i| var_name(step_ix, i as u16))
                    .collect();
                let define_values_expr = create_define_values_expr(output_vars, node_fn_call_expr);
                stmts.push(define_values_expr);
            }
        }
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

/// Collect all unique node configurations for all unique evaluation paths that
/// exist in the graph.
fn node_confs<'a, I>(eval_stepss: I) -> BTreeSet<(node::Id, Vec<bool>)>
where
    I: IntoIterator<Item = &'a [EvalStep]>,
{
    eval_stepss
        .into_iter()
        .flat_map(|steps| {
            steps.iter().map(|step| {
                let conf = step.args.iter().map(|arg| arg.is_some()).collect();
                (step.node, conf)
            })
        })
        .collect()
}

/// Construct a rose tree of node configs from a tree of eval plans.
fn node_confs_tree(eval_tree: &RoseTree<EvalPlan>) -> RoseTree<NodeConfs> {
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

/// The string used to represent a path in a fn name.
fn path_string(path: &[node::Id]) -> String {
    path.iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join("_")
}

/// Generate a function name for a node based on its path in the graph.
fn node_fn_name(node_path: &[node::Id], inputs: &[bool]) -> String {
    let path_string = path_string(node_path);
    let inputs_prefix = if inputs.is_empty() { "" } else { "_i" };
    let inputs_string = inputs
        .iter()
        .map(|&b| if b { "1" } else { "0" })
        .collect::<Vec<_>>()
        .join("");
    format!("node_fn_{path_string}{inputs_prefix}{inputs_string}")
}

/// Generate a function for a single node with the given set of connected inputs.
fn node_fn(node: &dyn Node, node_path: &[node::Id], inputs: &[bool]) -> ExprKind {
    // The binding used to receive the node's state as an argument, and whose
    // resulting value is returned from the body of the function and used to
    // update the state map.
    const STATE: &str = "state";

    fn input_name(i: usize) -> String {
        format!("input{i}")
    }

    // Create function parameters for graph state and inputs
    let mut input_args = inputs
        .iter()
        .enumerate()
        .filter_map(|(i, b)| b.then(|| input_name(i)))
        .collect::<Vec<_>>();

    // Create input expressions for the node's expr method
    let input_exprs: Vec<Option<String>> = inputs
        .iter()
        .enumerate()
        .map(|(i, b)| b.then(|| input_name(i)))
        .collect();

    // Get the node's expression
    let ctx = node::ExprCtx::new(node_path, &input_exprs);
    let node_expr = node.expr(ctx);

    // Construct the full function definition
    let fn_name = node_fn_name(node_path, inputs);
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
fn node_fns<G>(g: G, node_confs_tree: &RoseTree<NodeConfs>) -> Vec<ExprKind>
where
    G: Data<EdgeWeight = Edge> + IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable,
    G::NodeWeight: Node,
{
    let mut node_fns = NodeFns::new(&node_confs_tree);
    crate::graph::visit(g, &[], &mut node_fns);
    node_fns.fns
}

/// Given a tree of eval plans for a gantz graph (and its nested graphs),
/// generate all push, pull and nested eval fns for the graph.
fn eval_fns(eval_tree: &RoseTree<EvalPlan>) -> Vec<ExprKind> {
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
            let stmts = eval_stmts(path, &steps, &eval.flow.outputs, &eval.flow.stateful);
            eval_fn(&name, stmts)
        });
        eval_fns.extend(fns);
    });
    eval_fns
}

/// Given a root gantz graph, generate the full module with all the necessary
/// functions for executing it.
///
/// This includes:
///
/// 1. A function for each node (and for each node input configuration).
/// 2. A function for each node requiring push/pull evaluation.
/// 3. The above for all nested graphs.
pub fn module<G>(g: G) -> Vec<ExprKind>
where
    G: Data<EdgeWeight = Edge> + IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable,
    G::NodeWeight: Node,
{
    // Create a `Flow` for each graph (including nested) in a tree.
    let mut flow_tree = RoseTree::<Flow>::default();
    crate::graph::visit(g, &[], &mut flow_tree);
    let eval_tree = flow_tree.map_ref(&mut eval_plan);

    // Collect node fns.
    let node_confs_tree = node_confs_tree(&eval_tree);
    let node_fns = node_fns(g, &node_confs_tree);

    // Collect eval fns.
    let eval_fns = eval_fns(&eval_tree);

    node_fns.into_iter().chain(eval_fns).collect()
}
