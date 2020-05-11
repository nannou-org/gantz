use super::Edge;
use crate::node::{self, Node};
use petgraph::visit::{
    Data, Dfs, EdgeRef, GraphRef, IntoEdgesDirected, IntoNodeReferences, NodeIndexable, NodeRef,
    Topo, Visitable, Walker,
};
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use syn::punctuated::Punctuated;

/// An evaluation step ready for translation to rust code.
#[derive(Debug)]
pub struct EvalStep<NI> {
    /// The node to be evaluated.
    pub node: NI,
    /// Arguments to the node's function call.
    ///
    /// The `len` of the outer vec will always be equal to the number of inputs on `node`.
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

/// Shorthand for the node evaluator map passed between codegen stages.
pub type NodeEvaluatorMap<Id> = HashMap<Id, node::Evaluator>;

/// Shorthand for the node state type map passed between codegen stages.
pub type NodeStateTypeMap<Id> = HashMap<Id, syn::Type>;

/// Given a graph of gantz nodes, produce the `Evaluator` associated with each.
pub fn node_evaluators<G>(g: G) -> NodeEvaluatorMap<G::NodeId>
where
    G: IntoNodeReferences,
    G::NodeWeight: Node,
    G::NodeId: Eq + Hash,
{
    g.node_references()
        .map(|n| (n.id(), n.weight().evaluator()))
        .collect()
}

/// Given a graph of gantz nodes, produce a map from stateful node IDs to their associated state
/// types.
pub fn node_state_types<G>(g: G) -> NodeStateTypeMap<G::NodeId>
where
    G: IntoNodeReferences,
    G::NodeWeight: Node,
    G::NodeId: Eq + Hash,
{
    g.node_references()
        .filter_map(|n| n.weight().state_type().map(|ty| (n.id(), ty)))
        .collect()
}

/// Given a set of node evaluators, return only those that have function definitions.
///
/// Evaluator function variants associated with stateful nodes will have a state argument
/// implicitly added to their function signature.
pub fn node_evaluator_fns<'a, Id>(
    state_types: &'a NodeStateTypeMap<Id>,
    evaluators: &'a NodeEvaluatorMap<Id>,
) -> impl 'a + Iterator<Item = (&'a Id, syn::ItemFn)>
where
    Id: 'a + Eq + Hash,
{
    evaluators.iter().filter_map(move |(id, eval)| match eval {
        node::Evaluator::Fn { ref fn_item } => {
            let mut fn_item = fn_item.clone();
            if let Some(ty) = state_types.get(&id) {
                fn_item
                    .sig
                    .inputs
                    .push(syn::parse_quote! { state: &mut #ty });
            }
            Some((id, fn_item))
        }
        node::Evaluator::Expr { .. } => None,
    })
}

/// Given a graph of gantz nodes, return `NodeId`s of those that require push evaluation.
///
/// Expects any graph type whose nodes implement `Node`.
pub fn push_nodes<G>(g: G) -> Vec<(G::NodeId, node::EvalFn)>
where
    G: IntoNodeReferences,
    G::NodeWeight: Node,
{
    g.node_references()
        .filter_map(|n| n.weight().push_eval().map(|eval| (n.id(), eval)))
        .collect()
}

/// Given a graph of gantz nodes, return `NodeId`s of those that require pull evaluation.
///
/// Expects any graph type whose nodes implement `Node`.
pub fn pull_nodes<G>(g: G) -> Vec<(G::NodeId, node::EvalFn)>
where
    G: IntoNodeReferences,
    G::NodeWeight: Node,
{
    g.node_references()
        .filter_map(|n| n.weight().pull_eval().map(|eval| (n.id(), eval)))
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
/// Evaluation order is equivalent to a topological ordering of the connected component
/// starting from the given node.
///
/// Expects any directed graph whose edges are of type `Edge` and whose nodes implement `Node`.
/// Direction of edges indicate the flow of data through the graph.
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
/// Evaluation order is equivalent to a topological ordering of the connected component that
/// ends at the given node.
///
/// Expects any directed graph whose edges are of type `Edge` and whose nodes implement `Node`.
/// Direction of edges indicate the flow of data through the graph.
pub fn pull_eval_order<G>(g: G, n: G::NodeId) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + IntoNodeReferences + Visitable,
    G::NodeId: Eq + Hash,
{
    let dfs: HashSet<G::NodeId> = pull_reachable(g, n).collect();
    Topo::new(g).iter(g).filter(move |node| dfs.contains(&node))
}

/// The evaluation order for given any number of simultaneously pushing and pulling nodes.
///
/// Evaluation order is equivalent to a topological ordering of the connected components reachable
/// via DFS from each push node and reversed-edge DFS from each pull node.
///
/// Expects any directed graph whose edges are of type `Edge` and whose nodes implement `Node`.
/// Direction of edges indicate the flow of data through the graph.
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

/// Given a node evaluation order, this filters out all non-stateful nodes.
///
/// This order of the yielded node IDs matches the expected order in which state should be laid out
/// when passed to the generated evaluation function.
pub fn state_order<G, I>(g: G, eval_order: I) -> impl Iterator<Item = G::NodeId>
where
    G: IntoNodeReferences + NodeIndexable,
    I: IntoIterator<Item = G::NodeId>,
    G::NodeWeight: Node,
{
    eval_order.into_iter().filter(move |&n| {
        g.node_references()
            .nth(g.to_index(n))
            .expect("node in `eval_order` does not exist within the given graph")
            .weight()
            .state_type()
            .is_some()
    })
}

/// Given a node evaluation order, produce the series of evaluation steps required.
pub fn eval_steps<G, I>(
    g: G,
    node_evaluators: &NodeEvaluatorMap<G::NodeId>,
    eval_order: I,
) -> Vec<EvalStep<G::NodeId>>
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
        let child_evaluator = &node_evaluators[&node];
        let mut args: Vec<_> = (0..child_evaluator.n_inputs()).map(|_| None).collect();

        // Create an argument for each input to this child.
        for e_ref in g.edges_directed(node, petgraph::Incoming) {
            // Only consider edges to nodes that we have already visited.
            if !visited.contains(&e_ref.source()) {
                continue;
            }

            let w = e_ref.weight();

            // Check how many connections their are from the parent's output and see if the
            // value will need to be cloned when passed to this input.
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

/// Given a function argument, return its type if known.
pub fn ty_from_fn_arg(arg: &syn::FnArg) -> Option<syn::Type> {
    match arg {
        syn::FnArg::Typed(ref pat_ty) => Some((*pat_ty.ty).clone()),
        _ => None,
    }
}

/// Generate a sequence of evaluation statements, one for each given evaluation step.
pub fn eval_stmts<G>(
    g: G,
    steps: &[EvalStep<G::NodeId>],
    node_state_types: &NodeStateTypeMap<G::NodeId>,
    node_evaluators: &NodeEvaluatorMap<G::NodeId>,
) -> Vec<syn::Stmt>
where
    G: GraphRef + IntoNodeReferences + NodeIndexable,
    G::NodeId: Eq + Hash,
    G::NodeWeight: Node,
{
    type LValues<NI> = HashMap<(NI, node::Output), syn::Ident>;

    // A function for constructing a variable name.
    fn var_name(node_ix: usize, out_ix: u32) -> String {
        format!("_node{}_output{}", node_ix, out_ix)
    }

    // Insert the lvalue for the node output with the given name into the given map.
    fn insert_lvalue<NI>(node_id: NI, out_ix: u32, name: &str, lvals: &mut LValues<NI>)
    where
        NI: Eq + Hash,
    {
        let output = node::Output(out_ix);
        let ident = syn::Ident::new(name, proc_macro2::Span::call_site());
        lvals.insert((node_id, output), ident);
    };

    // Construct a pattern for a function argument.
    fn var_pat(name: &str) -> syn::Pat {
        let ident = syn::Ident::new(name, proc_macro2::Span::call_site());
        let pat_ident = syn::PatIdent {
            attrs: vec![],
            by_ref: None,
            mutability: None,
            subpat: None,
            ident,
        };
        syn::Pat::Ident(pat_ident)
    }

    // Retrieve the expr for the input to the function.
    fn input_expr<G>(
        g: G,
        arg: Option<&ExprInput<G::NodeId>>,
        lvals: &LValues<G::NodeId>,
    ) -> syn::Expr
    where
        G: NodeIndexable,
        G::NodeId: Eq + Hash,
    {
        match arg {
            None => syn::parse_quote! { () },
            Some(arg) => {
                let ident = lvals.get(&(arg.node, arg.output)).unwrap_or_else(|| {
                    panic!(
                        "no lvalue for expected arg (node {}, output {})",
                        g.to_index(arg.node),
                        arg.output.0,
                    );
                });
                match arg.requires_clone {
                    false => syn::parse_quote! { { #ident } },
                    true => syn::parse_quote! { { #ident.clone() } },
                }
            }
        }
    }

    // Create the lvals pattern, either `PatWild` for no outputs, `Ident` for single output or
    // `Tuple` for multiple.
    //
    // Also keeps track of each the lvalue ident for each output of the node so that they may be
    // passed to following node exprs.
    fn lvalues_pat<Id>(
        step_ix: usize,
        step: &EvalStep<Id>,
        n_outputs: u32,
        lvalues: &mut LValues<Id>,
    ) -> syn::Pat
    where
        Id: Copy + Eq + Hash,
    {
        let v_name = |vi| var_name(step_ix, vi);
        let mut insert_lval = |vi, name: &str| {
            insert_lvalue(step.node, vi, name, lvalues);
        };
        match n_outputs {
            0 => syn::parse_quote! { () },
            1 => {
                let vi = 0;
                let v = v_name(vi);
                insert_lval(vi, &v);
                var_pat(&v)
            }
            vs => {
                let punct = (0..vs)
                    .map(|vi| {
                        let v = v_name(vi);
                        insert_lval(vi, &v);
                        var_pat(&v)
                    })
                    .collect::<Punctuated<syn::Pat, syn::Token![,]>>();
                syn::parse_quote! { (#punct) }
            }
        }
    }

    // For each evaluation step, generate a statement where the expression for the node at that
    // evaluation step is evaluated and the outputs are destructured from a tuple.
    let mut stmts: Vec<syn::Stmt> = vec![];

    // Keep track of each of the lvalues for each of the statements.
    let mut lvalues: LValues<G::NodeId> = Default::default();

    // The next index into the node_states slice.
    //
    // TODO: This should be calculated ahead of this function in an easy-to-access manner so that
    // users can pre-prepare node states properly.
    let mut node_state_idx: usize = 0;

    for (si, step) in steps.iter().enumerate() {
        // Retrieve an expression for each argument to the current node's expression.
        //
        // E.g. `_n1_v0`, `_n3_v1.clone()` or `Default::default()`.
        let args: Vec<syn::Expr> = step
            .args
            .iter()
            .map(|arg| input_expr(g, arg.as_ref(), &lvalues))
            .collect();
        let ne = &node_evaluators[&step.node];
        let n_outputs = ne.n_outputs();
        let lhs: syn::Pat = lvalues_pat(si, step, n_outputs, &mut lvalues);
        let n_id = g
            .node_references()
            .nth(g.to_index(step.node))
            .expect("no node for step's node index")
            .id();
        let maybe_state_ty = node_state_types.get(&n_id);
        let expr: syn::Expr = ne.expr(args, maybe_state_ty.is_some());

        let rhs: syn::Expr = match maybe_state_ty {
            None => expr,
            Some(node_state_ty) => {
                let expr = syn::parse_quote! {{
                    let state: &mut #node_state_ty = _node_states[#node_state_idx]
                        .downcast_mut::<#node_state_ty>()
                        .expect("failed to downcast to expected node state type");
                    #expr
                }};
                node_state_idx += 1;
                expr
            }
        };

        let stmt: syn::Stmt = syn::parse_quote! {
            let #lhs = #rhs;
        };

        stmts.push(stmt);
    }

    stmts
}

/// Generate a function for performing evaluation of the given statements.
///
/// The given `Vec<syn::Stmt>` should be generated via the `eval_stmts` function.
///
/// This function modifies given `EvalFn` fields in two ways:
///
/// - Adds a `#[no_mangle]` attribute if one doesn't already exist.
/// - Adds a `_node_states: &mut [&mut dyn std::any::Any]` input to the function signature to
///   allow for passing through unique state associated with each node.
pub fn eval_fn(eval_fn: node::EvalFn, stmts: Vec<syn::Stmt>) -> syn::ItemFn {
    let brace_token = Default::default();
    let block = Box::new(syn::Block { stmts, brace_token });

    let node::EvalFn {
        mut fn_attrs,
        mut signature,
    } = eval_fn;

    // Add the `#[no_mangle]` attr to the function so that the symbol retains its name.
    let no_mangle = no_mangle_attr();
    if !fn_attrs.iter().any(|attr| *attr == no_mangle) {
        fn_attrs.push(no_mangle);
    }

    // Append the argument for acquiring node states.
    let node_states = node_states_fn_arg();
    if !signature.inputs.iter().any(|input| *input == node_states) {
        signature.inputs.push(node_states);
    }

    let sig = signature;
    let pub_token = Default::default();
    let vis = syn::Visibility::Public(syn::VisPublic { pub_token });

    let item_fn = syn::ItemFn {
        attrs: fn_attrs,
        vis,
        sig,
        block,
    };

    item_fn
}

/// Given a list of push evaluation nodes and their evaluation steps, generate a function for
/// performing push evaluation for each node.
pub fn eval_fns<'a, G, I>(
    g: G,
    eval_nodes: I,
    node_state_types: &NodeStateTypeMap<G::NodeId>,
    node_evaluators: &NodeEvaluatorMap<G::NodeId>,
) -> Vec<syn::ItemFn>
where
    G: GraphRef + IntoNodeReferences + NodeIndexable,
    G::NodeId: 'a + Eq + Hash,
    G::NodeWeight: Node,
    I: IntoIterator<Item = (G::NodeId, node::EvalFn, &'a [EvalStep<G::NodeId>])>,
{
    eval_nodes
        .into_iter()
        .map(|(_n, eval, steps)| {
            let stmts = eval_stmts(g, steps, node_state_types, node_evaluators);
            eval_fn(eval, stmts)
        })
        .collect()
}

/// Given a gantz graph, generate the rust code src file with all the necessary functions for
/// executing it.
pub fn file<G>(g: G, inlets: &[G::NodeId], outlets: &[G::NodeId]) -> syn::File
where
    G: GraphRef + IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable,
    G: Data<EdgeWeight = Edge>,
    G::NodeId: Eq + Hash,
    G::NodeWeight: Node,
{
    let node_state_types = node_state_types(g);
    let node_evaluators = node_evaluators(g);
    let node_evaluator_fn_items = node_evaluator_fns(&node_state_types, &node_evaluators);

    let full_eval_steps = match (inlets.is_empty(), outlets.is_empty()) {
        (true, true) => None,
        _ => {
            let eval = super::full_eval_fn();
            let order = eval_order(g, inlets.iter().cloned(), outlets.iter().cloned());
            let steps = eval_steps(g, &node_evaluators, order);
            Some((steps, eval))
        }
    };

    let pull_nodes = pull_nodes(g);
    let push_nodes = push_nodes(g);
    let pull_node_eval_steps = pull_nodes.into_iter().map(|(n, eval)| {
        let order = pull_eval_order(g, n);
        let steps = eval_steps(g, &node_evaluators, order);
        (steps, eval)
    });
    let push_node_eval_steps = push_nodes.into_iter().map(|(n, eval)| {
        let order = push_eval_order(g, n);
        let steps = eval_steps(g, &node_evaluators, order);
        (steps, eval)
    });
    let all_eval_steps = full_eval_steps
        .into_iter()
        .chain(pull_node_eval_steps)
        .chain(push_node_eval_steps);
    let all_eval_fn_items = all_eval_steps.map(|(steps, eval)| {
        let stmts = eval_stmts(g, &steps, &node_state_types, &node_evaluators);
        let item_fn = eval_fn(eval, stmts);
        syn::Item::Fn(item_fn)
    });

    let items = node_evaluator_fn_items
        .map(|(_, item_fn)| syn::Item::Fn(item_fn.clone()))
        .chain(all_eval_fn_items)
        .collect();

    let attrs = vec![allow_unused_braces_attr()];

    let file = syn::File {
        shebang: None,
        attrs,
        items,
    };
    file
}

/// The total set of crate dependencies required for all nodes within the given graph.
///
/// This is useful for filling the `[dependencies]` entry of a generated crate's `Cargo.toml`.
pub fn crate_deps<G>(g: G) -> HashSet<node::CrateDep>
where
    G: IntoNodeReferences,
    G::NodeWeight: Node,
{
    g.node_references()
        .flat_map(|n| n.weight().crate_deps())
        .collect()
}

fn no_mangle_attr() -> syn::Attribute {
    syn::parse_quote! { #[no_mangle] }
}

fn allow_unused_braces_attr() -> syn::Attribute {
    syn::parse_quote! { #![allow(unused_braces)] }
}

// A function that produces the `node_states` argument that is to be appended to the evaluation
// function `inputs`.
//
// TODO: Eventually we should use a more user-friendly type for passing through state.
fn node_states_fn_arg() -> syn::FnArg {
    syn::parse_quote! { _node_states: &mut [&mut dyn std::any::Any] }
}
