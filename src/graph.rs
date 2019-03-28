use super::{Deserialize, Serialize};
use crate::node::{self, Node};

/// The type used to represent node and edge indices.
pub type Index = usize;

pub type EdgeIndex = petgraph::graph::EdgeIndex<Index>;
pub type NodeIndex = petgraph::graph::NodeIndex<Index>;

/// Describes a connection between two nodes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct Edge {
    /// The output of the node at the source of this edge.
    pub output: node::Output,
    /// The input of the node at the destination of this edge.
    pub input: node::Input,
}

/// The petgraph type used to represent a gantz graph.
pub type Graph<N> = petgraph::Graph<N, Edge, petgraph::Directed, Index>;

/// The petgraph type used to represent a stable gantz graph.
pub type StableGraph<N> = petgraph::stable_graph::StableGraph<N, Edge, petgraph::Directed, Index>;

impl Edge {
    /// Create an edge representing a connection from the given node `Output` to the given node
    /// `Input`.
    pub fn new(output: node::Output, input: node::Input) -> Self {
        Edge { output, input }
    }
}

impl<N> Node for StableGraph<N>
where
    N: Node,
{
    fn n_inputs(&self) -> u32 {
        unimplemented!("requires implementing graph inlet nodes")
    }

    fn n_outputs(&self) -> u32 {
        unimplemented!("requires implementing graph outlet nodes")
    }

    fn expr(&self, _args: Vec<syn::Expr>) -> syn::Expr {
        unimplemented!("requires implementing graph inlet and outlet nodes")
    }
}

impl<A, B> From<(A, B)> for Edge
where
    A: Into<node::Output>,
    B: Into<node::Input>,
{
    fn from((a, b): (A, B)) -> Self {
        let output = a.into();
        let input = b.into();
        Edge { output, input }
    }
}

pub mod codegen {
    use crate::node::{self, Node};
    use petgraph::visit::{Data, EdgeRef, GraphRef, IntoEdgesDirected, IntoNodeReferences,
                          NodeIndexable, NodeRef, Visitable, Walker};
    use std::collections::{HashMap, HashSet};
    use std::hash::Hash;
    use super::Edge;
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

    /// Given a graph with of gantz nodes, return `NodeId`s of those that require push evaluation.
    ///
    /// Expects any graph type whose nodes implement `Node`.
    pub fn push_nodes<G>(g: G) -> Vec<(G::NodeId, node::PushEval)>
    where
        G: IntoNodeReferences,
        <G::NodeRef as NodeRef>::Weight: Node,
    {
        g.node_references()
            .filter_map(|n| n.weight().push_eval().map(|eval| (n.id(), eval)))
            .collect()
    }

    /// Given a graph with of gantz nodes, return `NodeId`s of those that require pull evaluation.
    ///
    /// Expects any graph type whose nodes implement `Node`.
    pub fn pull_nodes<G>(g: G) -> Vec<(G::NodeId, node::PullEval)>
    where
        G: IntoNodeReferences,
        <G::NodeRef as NodeRef>::Weight: Node,
    {
        g.node_references()
            .filter_map(|n| n.weight().pull_eval().map(|eval| (n.id(), eval)))
            .collect()
    }

    /// Push evaluation from the specified node.
    ///
    /// Evaluation order is equivalent to depth-first-search post order.
    ///
    /// Expects any directed graph whose edges are of type `Edge` and whose nodes implement `Node`.
    /// Direction of edges indicate the flow of data through the graph.
    pub fn push_eval_steps<G>(g: G, n: G::NodeId) -> Vec<EvalStep<G::NodeId>>
    where
        G: GraphRef + IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable,
        G: Data<EdgeWeight = Edge>,
        G::NodeId: Eq + Hash,
        <G::NodeRef as NodeRef>::Weight: Node,
    {
        // First, find all nodes reachable by a `DFS` from this node.
        let dfs: HashSet<G::NodeId> = petgraph::visit::Dfs::new(g, n).iter(g).collect();

        // The order of evaluation is DFS post order.
        let mut traversal = petgraph::visit::Topo::new(g);

        // Track the evaluation steps.
        let mut eval_steps = vec![];

        // Step through each of the nodes.
        while let Some(node) = traversal.next(g) {
            if !dfs.contains(&node) {
                continue;
            }

            // Fetch the node reference.
            let child = g.node_references()
                .nth(g.to_index(node))
                .expect("no node for index");

            // Initialise the arguments to `None` for each input.
            let mut args: Vec<_> = (0..child.weight().n_inputs()).map(|_| None).collect();

            // Create an argument for each input to this child. 
            for e_ref in g.edges_directed(node, petgraph::Incoming) {
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

    /// Pull evaluation from the specified node.
    ///
    /// Evaluation order is equivalent to depth-first-search order, ending with the specified node.
    ///
    /// Expects any directed graph whose edges are of type `Edge` and whose nodes implement `Node`.
    /// Direction of edges indicate the flow of data through the graph.
    pub fn pull_eval_steps<G>(g: G, n: G::NodeId) -> Vec<EvalStep<G::NodeId>>
    where
        G: GraphRef + IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable,
        G: Data<EdgeWeight = Edge>,
        <G::NodeRef as NodeRef>::Weight: Node,
    {
        // TODO
        unimplemented!()
    }

    /// Given a function argument, return its type if known.
    pub fn ty_from_fn_arg(arg: &syn::FnArg) -> Option<syn::Type> {
        match arg {
            syn::FnArg::Captured(cap) => Some(cap.ty.clone()),
            syn::FnArg::Ignored(ty) => Some(ty.clone()),
            _ => None,
        }
    }

    /// Generate a function for performing push evaluation from the given node with the given
    /// evaluation steps.
    pub fn push_eval_fn<G>(
        g: G,
        push_eval: node::PushEval,
        steps: &[EvalStep<G::NodeId>],
    ) -> syn::ItemFn
    where
        G: GraphRef + IntoNodeReferences + NodeIndexable,
        G::NodeId: Eq + Hash,
        <G::NodeRef as NodeRef>::Weight: Node,
    {
        // For each evaluation step, generate a statement where the expression for the node at that
        // evaluation step is evaluated and the outputs are destructured from a tuple.
        let mut stmts: Vec<syn::Stmt> = vec![];

        // Keep track of each of the lvalues for each of the statements. These are used to pass
        let mut lvalues: HashMap<(G::NodeId, node::Output), syn::Ident> = Default::default();

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
                None => syn::parse_quote! { Default::default() },
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

        for (si, step) in steps.iter().enumerate() {
            let n_ref = g.node_references().nth(g.to_index(step.node)).expect("no node for index");

            // Retrieve an expression for each argument to the current node's expression.
            //
            // E.g. `_n1_v0`, `_n3_v1.clone()` or `Default::default()`.
            let args: Vec<syn::Expr> = step.args.iter()
                .map(|arg| input_expr(g, arg.as_ref(), &lvalues))
                .collect();

            let nw = n_ref.weight();
            let n_outputs = nw.n_outputs();
            let expr: syn::Expr = nw.expr(args);

            // Create the lvals pattern, either `PatWild` for no outputs, `Ident` for single output
            // or `Tuple` for multiple. Keep track of each the lvalue ident for each output of the
            // node so that they may be passed to following node exprs.
            let lvals: syn::Pat = {
                let v_name = |vi| var_name(si, vi);
                let mut insert_lval = |vi, name: &str| {
                    insert_lvalue(step.node, vi, name, &mut lvalues);
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
            };

            let stmt: syn::Stmt = syn::parse_quote!{
                let #lvals = #expr;
            };

            stmts.push(stmt);
        }

        // Construct the final function item.
        let block = Box::new(syn::Block { stmts, brace_token: Default::default() });
        let node::PushEval { fn_decl, fn_name, mut fn_attrs } = push_eval;
        let decl = Box::new(fn_decl);
        let ident = syn::Ident::new(&fn_name, proc_macro2::Span::call_site());
        let vis = syn::Visibility::Public(syn::VisPublic { pub_token: Default::default() });

        // Add the `#[no_mangle]` attr to the function so that the symbol retains its name.
        let no_mangle = no_mangle_attr();
        if !fn_attrs.iter().any(|attr| *attr == no_mangle) {
            fn_attrs.push(no_mangle);
        }

        let item_fn = syn::ItemFn {
            attrs: fn_attrs,
            vis,
            constness: None,
            unsafety: None,
            asyncness: None,
            abi: None,
            ident,
            decl,
            block,
        };

        item_fn
    }

    /// Given a list of push evaluation nodes and their evaluation steps, generate a function for
    /// performing push evaluation for each node.
    pub fn push_eval_fns<'a, G, I>( g: G, push_eval_nodes: I,) -> Vec<syn::ItemFn>
    where
        G: GraphRef + IntoNodeReferences + NodeIndexable,
        G::NodeId: 'a + Eq + Hash,
        <G::NodeRef as NodeRef>::Weight: Node,
        I: IntoIterator<Item = (G::NodeId, node::PushEval, &'a [EvalStep<G::NodeId>])>,
    {
        push_eval_nodes
            .into_iter()
            .map(|(_n, eval, steps)| push_eval_fn(g, eval, steps))
            .collect()
    }

    /// Generate a function for performing pull evaluation from the given node with the given
    /// evaluation steps.
    pub fn pull_eval_fn<G>(
        g: G,
        pull_eval: node::PullEval,
        steps: &[EvalStep<G::NodeId>],
    ) -> syn::ItemFn
    where
        G: GraphRef + IntoNodeReferences + NodeIndexable,
        G::NodeId: Eq + Hash,
        <G::NodeRef as NodeRef>::Weight: Node,
    {
        // TODO
        unimplemented!();
    }

    /// Given a gantz graph, generate the rust code src file with all the necessary functions for
    /// executing it.
    pub fn file<G>(g: G) -> syn::File
    where
        G: GraphRef + IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable,
        G: Data<EdgeWeight = Edge>,
        G::NodeId: Eq + Hash,
        <G::NodeRef as NodeRef>::Weight: Node,
    {
        let push_nodes = push_nodes(g);
        let items = push_nodes
            .into_iter()
            .map(|(n, eval)| {
                let steps = push_eval_steps(g, n);
                let item_fn = push_eval_fn(g, eval, &steps);
                syn::Item::Fn(item_fn)
            })
            .collect();
        let file = syn::File { shebang: None, attrs: vec![], items };
        file
    }

    // Create the `#[no_mangle]` attribute.
    fn no_mangle_attr() -> syn::Attribute {
        let ident = syn::Ident::new("no_mangle", proc_macro2::Span::call_site());
        let arguments = syn::PathArguments::None;
        let segments = Some(syn::PathSegment { ident, arguments }).into_iter().collect();
        let path = syn::Path { leading_colon: None, segments };
        let style = syn::AttrStyle::Outer;
        let pound_token = Default::default();
        let bracket_token = Default::default();
        let tts = Default::default();
        syn::Attribute { pound_token, style, bracket_token, path, tts }
    }
}
