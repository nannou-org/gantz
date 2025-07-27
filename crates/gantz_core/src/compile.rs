//! Items related to generating steel code from a gantz graph, primarily the
//! [`module`] fn.

use crate::{
    Edge, GRAPH_STATE, ROOT_STATE,
    node::{self, Node},
};
#[doc(inline)]
pub use meta::Meta;
use node_fns::{node_confs_tree, node_fn_name, node_fns};
use petgraph::visit::{
    Data, Dfs, EdgeRef, GraphBase, GraphRef, IntoEdgesDirected, IntoNeighbors, IntoNodeReferences,
    NodeIndexable, Topo, Visitable, Walker,
};
pub(crate) use rosetree::RoseTree;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    hash::Hash,
};
use steel::{parser::ast::ExprKind, steel_vm::engine::Engine};

mod meta;
mod node_fns;
mod rosetree;

/// Allows for remaining generic over graph type edges that represent one or
/// more gantz [`Edge`]s.
///
/// This is necessary to support calling the `reachable` fns with both the
/// `Meta` graph and graphs of different types.
pub trait Edges {
    /// Produce an iterator yielding all the [`Edge`]s.
    fn edges(&self) -> impl Iterator<Item = Edge>;
}

/// A representation of how to evaluate a graph.
///
/// Produced via [`eval_plan`].
#[derive(Debug)]
struct EvalPlan<'a> {
    /// The gantz graph `Meta` from which this `EvalPlan` was produced.
    meta: &'a Meta,

    /// Order of evaluation from all inlets to all outlets.
    ///
    /// Empty in the case that the graph has no inlets or outlets (i.e. is not
    /// nested).
    // TODO: Knowing the connectedness of the inlets/outlets would be useful
    // for generating only the necessary node configs.
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
    pub(crate) inputs: Vec<Option<ExprInput>>,
    /// The set of connected outputs.
    pub(crate) outputs: Vec<bool>,
}

/// An argument to a node's function call.
#[derive(Debug)]
pub(crate) struct ExprInput {
    /// The node from which the value was generated.
    pub(crate) node: node::Id,
    /// The output on the source node associated with the generated value.
    pub(crate) output: node::Output,
}

impl Edges for Edge {
    fn edges(&self) -> impl Iterator<Item = Edge> {
        std::iter::once(*self)
    }
}

impl Edges for Vec<Edge> {
    fn edges(&self) -> impl Iterator<Item = Edge> {
        self.iter().copied()
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

/// Given a graph and an eval src, return the set of direct neighbors that
/// will be included in the initial traversal.
fn eval_neighbors<G>(
    g: G,
    n: G::NodeId,
    conns: &node::Conns,
    src_conn: impl Fn(&Edge) -> usize,
) -> HashSet<G::NodeId>
where
    G: IntoEdgesDirected,
    G::EdgeWeight: Edges,
    G::NodeId: Eq + Hash,
{
    let mut set = HashSet::new();
    for e_ref in g.edges_directed(n, petgraph::Outgoing) {
        for edge in e_ref.weight().edges() {
            let conn_ix = src_conn(&edge);
            let include = conns.get(conn_ix).unwrap();
            if include {
                set.insert(e_ref.target());
            }
        }
    }
    set
}

/// Given a graph and a `EvalConf` src, return the set of direct neighbors that
/// will be included in the initial traversal.
fn push_eval_neighbors<G>(g: G, n: G::NodeId, ev: &node::Conns) -> HashSet<G::NodeId>
where
    G: IntoEdgesDirected,
    G::EdgeWeight: Edges,
    G::NodeId: Eq + Hash,
{
    eval_neighbors(g, n, ev, |edge| edge.output.0 as usize)
}

/// Given a graph and a `EvalConf` src, return the set of direct neighbors that
/// will be included in the initial traversal.
fn pull_eval_neighbors<G>(g: G, n: G::NodeId, ev: &node::Conns) -> HashSet<G::NodeId>
where
    G: IntoEdgesDirected,
    G::EdgeWeight: Edges,
    G::NodeId: Eq + Hash,
{
    let rev_g = petgraph::visit::Reversed(g);
    eval_neighbors(rev_g, n, ev, |edge| edge.input.0 as usize)
}

/// An iterator yielding all nodes reachable via pushing from the given node
/// over the given set of neighbors.
fn reachable<G>(
    g: G,
    src: G::NodeId,
    src_neighbors: &HashSet<G::NodeId>,
) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + Visitable,
    G::NodeId: Eq + Hash,
{
    /// A filter around a graph's `IntoNeighbors` implementation that discludes
    /// edges from the root that are not in the given src neighbors set.
    #[derive(Clone, Copy)]
    struct EvalFilter<'a, G: GraphBase> {
        g: G,
        /// The node that is the source of evaluation.
        src: G::NodeId,
        /// The eval src node's neighbors that are included in the eval set.
        src_neighbors: &'a HashSet<G::NodeId>,
    }

    /// The iterator filter applied to the inner graph's neighbors iterator.
    /// If we're iterating over the eval src's neighbors, this will only yield
    /// those specified in the included set.
    struct EvalFilterNeighbors<'a, I: Iterator> {
        neighbors: I,
        // Whether or not `neighbors` are from the eval src.
        is_src: bool,
        /// The eval src node's neighbors that are included in the eval set.
        src_neighbors: &'a HashSet<I::Item>,
    }

    impl<'a, G> GraphBase for EvalFilter<'a, G>
    where
        G: GraphBase,
    {
        type NodeId = G::NodeId;
        type EdgeId = G::EdgeId;
    }

    impl<'a, G: GraphRef> GraphRef for EvalFilter<'a, G> {}

    impl<'a, G> Visitable for EvalFilter<'a, G>
    where
        G: GraphRef + Visitable,
    {
        type Map = G::Map;
        fn visit_map(&self) -> Self::Map {
            self.g.visit_map()
        }
        fn reset_map(&self, map: &mut Self::Map) {
            self.g.reset_map(map);
        }
    }

    impl<'a, I> Iterator for EvalFilterNeighbors<'a, I>
    where
        I: Iterator,
        I::Item: Eq + Hash,
    {
        type Item = I::Item;
        fn next(&mut self) -> Option<Self::Item> {
            while let Some(n) = self.neighbors.next() {
                if !self.is_src || self.src_neighbors.contains(&n) {
                    return Some(n);
                }
            }
            None
        }
    }

    impl<'a, G> IntoNeighbors for EvalFilter<'a, G>
    where
        G: IntoNeighbors,
        G::NodeId: Eq + Hash,
    {
        type Neighbors = EvalFilterNeighbors<'a, G::Neighbors>;
        fn neighbors(self, a: Self::NodeId) -> Self::Neighbors {
            let neighbors = self.g.neighbors(a);
            EvalFilterNeighbors {
                neighbors,
                is_src: self.src == a,
                src_neighbors: &self.src_neighbors,
            }
        }
    }

    // Wrap the graph in a filter to only include src neighbors that appear in
    // the eval set.
    let g = EvalFilter {
        g,
        src,
        src_neighbors,
    };

    Dfs::new(g, src).iter(g)
}

/// An iterator yielding all nodes reachable via pushing from the given node
/// over the given set of direct neighbors.
fn push_reachable<G>(
    g: G,
    n: G::NodeId,
    nbs: &HashSet<G::NodeId>,
) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + Visitable,
    G::NodeId: Eq + Hash,
{
    reachable(g, n, nbs)
}

/// An iterator yielding all nodes reachable via pulling from the given node
/// over the given set of direct neighbors.
fn pull_reachable<G>(
    g: G,
    n: G::NodeId,
    nbs: &HashSet<G::NodeId>,
) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + Visitable,
    G::NodeId: Eq + Hash,
{
    let rev_g = petgraph::visit::Reversed(g);
    reachable(rev_g, n, nbs)
}

/// Push evaluation from the specified node.
///
/// Evaluation order is equivalent to a topological ordering of the connected
/// component starting from the given node.
///
/// Expects any directed graph whose edges are of type `Edge` and whose nodes
/// implement `Node`. Direction of edges indicate the flow of data through the
/// graph.
fn push_eval_order<G>(
    g: G,
    n: G::NodeId,
    nbs: &HashSet<G::NodeId>,
) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + IntoNodeReferences + Visitable,
    G::NodeId: Eq + Hash,
{
    let dfs: HashSet<G::NodeId> = push_reachable(g, n, nbs).collect();
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
fn pull_eval_order<G>(
    g: G,
    n: G::NodeId,
    nbs: &HashSet<G::NodeId>,
) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + IntoNodeReferences + Visitable,
    G::NodeId: Eq + Hash,
{
    let dfs: HashSet<G::NodeId> = pull_reachable(g, n, nbs).collect();
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
    G::EdgeWeight: Edges,
    G::NodeId: Eq + Hash,
    A: IntoIterator<Item = (G::NodeId, node::Conns)>,
    B: IntoIterator<Item = (G::NodeId, node::Conns)>,
{
    let mut reachable = HashSet::new();
    reachable.extend(push.into_iter().flat_map(|(n, conns)| {
        let ps = push_eval_neighbors(g, n, &conns);
        push_reachable(g, n, &ps).collect::<Vec<_>>()
    }));
    reachable.extend(pull.into_iter().flat_map(|(n, conns)| {
        let pl = pull_eval_neighbors(g, n, &conns);
        pull_reachable(g, n, &pl).collect::<Vec<_>>()
    }));
    Topo::new(g).iter(g).filter(move |n| reachable.contains(&n))
}

/// Given a node evaluation order, produce the series of evaluation steps
/// required.
pub(crate) fn eval_steps<I>(meta: &Meta, eval_order: I) -> impl Iterator<Item = EvalStep>
where
    I: IntoIterator<Item = node::Id>,
{
    // Step through each of the nodes.
    let mut visited = HashSet::new();
    eval_order.into_iter().map(move |n| {
        visited.insert(n);

        // Collect the inputs, initialising the set to `None`.
        let n_inputs = meta.inputs.get(&n).copied().unwrap_or(0);
        let mut inputs: Vec<_> = (0..n_inputs).map(|_| None).collect();
        for e_ref in meta.graph.edges_directed(n, petgraph::Incoming) {
            // Only consider edges to nodes that we have already visited.
            if !visited.contains(&e_ref.source()) {
                continue;
            }
            for (edge, _kind) in e_ref.weight() {
                // Assign the expression argument for this input.
                let arg = ExprInput {
                    node: e_ref.source(),
                    output: edge.output,
                };
                inputs[edge.input.0 as usize] = Some(arg);
            }
        }

        // Collect the set of connected outputs.
        let n_outputs = meta.outputs.get(&n).copied().unwrap_or(0);
        let mut outputs: Vec<_> = (0..n_outputs).map(|_| false).collect();
        for e_ref in meta.graph.edges_directed(n, petgraph::Outgoing) {
            for (edge, _kind) in e_ref.weight() {
                outputs[edge.output.0 as usize] |= true;
            }
        }

        EvalStep {
            node: n,
            inputs,
            outputs,
        }
    })
}

/// Create the evaluation plan for the graph associated with the given meta.
fn eval_plan(meta: &Meta) -> EvalPlan {
    let pull_steps = meta
        .pull
        .iter()
        .flat_map(|(&n, confs)| {
            confs.iter().map(move |conns| {
                let nbs = pull_eval_neighbors(&meta.graph, n, conns);
                let order = pull_eval_order(&meta.graph, n, &nbs);
                let steps = eval_steps(meta, order).collect();
                (n, steps)
            })
        })
        .collect();

    let push_steps = meta
        .push
        .iter()
        .flat_map(|(&n, confs)| {
            confs.iter().map(move |conns| {
                let nbs = push_eval_neighbors(&meta.graph, n, conns);
                let order = push_eval_order(&meta.graph, n, &nbs);
                let steps = eval_steps(meta, order).collect();
                (n, steps)
            })
        })
        .collect();

    let nested_steps = {
        let order = eval_order(
            &meta.graph,
            // FIXME: shouldn't hardcode these `Conns` counts...
            meta.inlets
                .iter()
                .map(|&n| (n, node::Conns::connected(1).unwrap())),
            meta.outlets
                .iter()
                .map(|&n| (n, node::Conns::connected(1).unwrap())),
        );
        eval_steps(meta, order).collect()
    };

    EvalPlan {
        meta,
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
pub fn eval_stmt(
    node_path: &[node::Id],
    inputs: &[Option<String>],
    outputs: &[bool],
    stateful: bool,
) -> (ExprKind, Vec<String>) {
    const STATE: &str = "state";

    // Function to generate variable names
    fn var_name(node_ix: node::Id, out_ix: u16) -> String {
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

    // The node index is the last element of the path.
    let node_ix = *node_path.last().expect("node_path must not be empty");

    // Create variables for this node's outputs
    let output_vars: Vec<_> = (0..outputs.len())
        .map(|i| var_name(node_ix, i as u16))
        .collect();

    let node_inputs: Vec<_> = inputs.iter().map(|arg| arg.is_some()).collect();
    let node_fn_name = node_fn_name(&node_path, &node_inputs, outputs);

    // Prepare function arguments.
    let mut args: Vec<String> = inputs.iter().filter_map(Clone::clone).collect();
    if stateful {
        args.push(STATE.to_string());
    }

    // The expression for the node function call.
    let mut node_fn_call_expr_str = format!("({node_fn_name} {})", args.join(" "));

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
        let (stmt, stmt_outputs) = eval_stmt(&node_path, &inputs, &step.outputs, stateful);

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
        .join("_")
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
            let stmts = eval_stmts(path, &steps, &eval.meta.stateful);
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
    // Create a `Meta` for each graph (including nested) in a tree.
    let mut meta_tree = RoseTree::<Meta>::default();
    crate::graph::visit(g, &[], &mut meta_tree);
    let eval_tree = meta_tree.map_ref(&mut eval_plan);

    // Collect node fns.
    let node_confs_tree = node_confs_tree(&eval_tree);
    let node_fns = node_fns(g, &node_confs_tree);

    // Collect eval fns.
    let eval_fns = eval_fns(&eval_tree);

    node_fns.into_iter().chain(eval_fns).collect()
}
