use crate::node::{self, Node, SerdeNode};
use petgraph::visit::GraphBase;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::ops::{Deref, DerefMut};
use syn::punctuated::Punctuated;
use syn::token::Comma;
use syn::FnArg;

/// The type used to represent node and edge indices.
pub type Index = usize;

pub type EdgeIndex = petgraph::graph::EdgeIndex<Index>;
pub type NodeIndex = petgraph::graph::NodeIndex<Index>;

/// A trait required by graphs that support nesting graphs of the same type as nodes.
pub trait EvaluatorFnBlock {
    /// The `Evaluator` function block used to evaluate the graph from its inputs to its outputs.
    ///
    /// The function declaration is provided in order to allow the implementer to inspect the
    /// function inputs and output and create a function body accordingly.
    fn evaluator_fn_block(&self, fn_decl: &syn::FnDecl) -> syn::Block;
}

/// Describes a connection between two nodes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct Edge {
    /// The output of the node at the source of this edge.
    pub output: node::Output,
    /// The input of the node at the destination of this edge.
    pub input: node::Input,
}

/// A node that itself is implemented in terms of a graph of nodes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GraphNode<G>
where
    G: GraphBase,
{
    /// The graph used to evaluate the node.
    pub graph: G,
    /// The types of each of the inputs into the graph node.
    ///
    /// TODO: Inlets and outlets should possibly use normal `Node`s and these should be their
    /// indices. This way we can retrieve the type from the graph, cast it to `Inlet`/`Outlet` and
    /// check for types while also allowing inlets and outlets to partake in the graph evaluation
    /// process.
    pub inlets: Vec<Inlet<G::NodeId>>,
    /// The types of each of the outputs into the graph node.
    pub outlets: Vec<Outlet<G::NodeId>>,
}

/// An inlet to a nested graph.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Inlet<Id> {
    /// The unique ID associated with this inlet's node in the graph.
    pub node_id: Id,
    /// The expected type for this inlet.
    #[serde(with = "crate::node::serde::ty")]
    pub ty: syn::Type,
}

/// An outlet from a nested graph.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Outlet<Id> {
    /// The unique ID associated with this outlet's node in the graph.
    pub node_id: Id,
    /// The expected type for this outlet.
    #[serde(with = "crate::node::serde::ty")]
    pub ty: syn::Type,
}

/// A node that may act as an inlet into a graph.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct InletNode;

/// A node that may act as an outlet from a graph.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct OutletNode;

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

impl<G> Node for GraphNode<G>
where
    G: GraphBase + EvaluatorFnBlock,
{
    fn evaluator(&self) -> node::Evaluator {
        let attrs = vec![];
        let vis = syn::Visibility::Inherited;
        let constness = None;
        let asyncness = None;
        let unsafety = None;
        let abi = None;
        // TODO: Make sure codegen makes the ident unique.
        // This will have to be considered in evaluator expr generation too.
        let name = format!("graph_node_evaluator_fn");
        let ident = syn::Ident::new(&name, proc_macro2::Span::call_site());
        let decl = Box::new(graph_node_evaluator_fn_decl(&self.inlets, &self.outlets));
        let block = Box::new(self.graph.evaluator_fn_block(&decl));
        let fn_item = syn::ItemFn {
            attrs,
            vis,
            constness,
            asyncness,
            unsafety,
            abi,
            ident,
            decl,
            block,
        };
        node::Evaluator::Fn { fn_item }
    }
}

// Manual implementation of `Deserialize` as it cannot be derived for a struct with associated
// types without unnecessary trait bounds on the struct itself.
impl<'de, G> Deserialize<'de> for GraphNode<G>
where
    G: GraphBase + Deserialize<'de>,
    G::NodeId: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::{self, MapAccess, SeqAccess, Visitor};

        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum Field {
            Graph,
            Inlets,
            Outlets,
        }

        struct GraphNodeVisitor<G>(std::marker::PhantomData<G>);

        impl<'de, G> Visitor<'de> for GraphNodeVisitor<G>
        where
            G: GraphBase + Deserialize<'de>,
            G::NodeId: Deserialize<'de>,
        {
            type Value = GraphNode<G>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("struct GraphNode")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<GraphNode<G>, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let graph = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let inlets = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let outlets = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(GraphNode {
                    graph,
                    inlets,
                    outlets,
                })
            }

            fn visit_map<V>(self, mut map: V) -> Result<GraphNode<G>, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut graph = None;
                let mut inlets = None;
                let mut outlets = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Graph => {
                            if graph.is_some() {
                                return Err(de::Error::duplicate_field("graph"));
                            }
                            graph = Some(map.next_value()?);
                        }
                        Field::Inlets => {
                            if inlets.is_some() {
                                return Err(de::Error::duplicate_field("inlets"));
                            }
                            inlets = Some(map.next_value()?);
                        }
                        Field::Outlets => {
                            if outlets.is_some() {
                                return Err(de::Error::duplicate_field("outlets"));
                            }
                            outlets = Some(map.next_value()?);
                        }
                    }
                }
                let graph = graph.ok_or_else(|| de::Error::missing_field("graph"))?;
                let inlets = inlets.ok_or_else(|| de::Error::missing_field("inlets"))?;
                let outlets = outlets.ok_or_else(|| de::Error::missing_field("outlets"))?;
                Ok(GraphNode {
                    graph,
                    inlets,
                    outlets,
                })
            }
        }

        const FIELDS: &[&str] = &["graph", "inlets", "outlets"];
        let visitor: GraphNodeVisitor<G> = GraphNodeVisitor(std::marker::PhantomData);
        deserializer.deserialize_struct("GraphNode", FIELDS, visitor)
    }
}

// Manual implementation of `Serialize` as it cannot be derived for a struct with associated
// types without unnecessary trait bounds on the struct itself.
impl<G> Serialize for GraphNode<G>
where
    G: GraphBase + Serialize,
    G::NodeId: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("GraphNode", 3)?;
        state.serialize_field("graph", &self.graph)?;
        state.serialize_field("inlets", &self.inlets)?;
        state.serialize_field("outlets", &self.outlets)?;
        state.end()
    }
}

impl Node for InletNode {
    fn evaluator(&self) -> node::Evaluator {
        let n_inputs = 1;
        let n_outputs = 1;
        //let ty = self.ty.clone();
        let gen_expr = Box::new(move |mut args: Vec<syn::Expr>| {
            assert_eq!(
                args.len(),
                1,
                "must be a single input (from the calling fn) for an inlet"
            );
            let in_expr = args.remove(0);
            syn::parse_quote! {
                //let in_expr_checked: #ty = #in_expr;
                //in_expr_checked
                #in_expr
            }
        });
        node::Evaluator::Expr {
            n_inputs,
            n_outputs,
            gen_expr,
        }
    }
}

impl Node for OutletNode {
    fn evaluator(&self) -> node::Evaluator {
        let n_inputs = 1;
        let n_outputs = 1;
        //let ty = self.ty.clone();
        let gen_expr = Box::new(move |mut args: Vec<syn::Expr>| {
            assert_eq!(
                args.len(),
                1,
                "must be a single input (from the calling fn) for an inlet"
            );
            let out_expr = args.remove(0);
            syn::parse_quote! {
                //let out_expr_checked: #ty = #in_expr;
                //out_expr_checked
                #out_expr
            }
        });
        node::Evaluator::Expr {
            n_inputs,
            n_outputs,
            gen_expr,
        }
    }
}

#[typetag::serde]
impl SerdeNode for InletNode {
    fn node(&self) -> &dyn Node {
        self
    }
}

#[typetag::serde]
impl SerdeNode for OutletNode {
    fn node(&self) -> &dyn Node {
        self
    }
}

impl<G> Deref for GraphNode<G>
where
    G: GraphBase,
{
    type Target = G;
    fn deref(&self) -> &Self::Target {
        &self.graph
    }
}

impl<G> DerefMut for GraphNode<G>
where
    G: GraphBase,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.graph
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

fn graph_node_evaluator_fn_decl<Id>(inlets: &[Inlet<Id>], outlets: &[Outlet<Id>]) -> syn::FnDecl {
    let fn_token = syn::token::Fn {
        span: proc_macro2::Span::call_site(),
    };
    let generics = {
        // TODO: Eventually we'll want some way of inspecting inlets/outlets for these.
        let lt_token = None;
        let params = syn::punctuated::Punctuated::new();
        let gt_token = None;
        let where_clause = None;
        syn::Generics {
            lt_token,
            params,
            gt_token,
            where_clause,
        }
    };
    let paren_token = syn::token::Paren {
        span: proc_macro2::Span::call_site(),
    };
    let variadic = None;
    let inputs = graph_node_evaluator_fn_inputs(inlets);
    let output = graph_node_evaluator_fn_output(outlets);
    syn::FnDecl {
        fn_token,
        generics,
        paren_token,
        inputs,
        variadic,
        output,
    }
}

fn graph_node_evaluator_fn_inputs<Id>(inlets: &[Inlet<Id>]) -> Punctuated<FnArg, Comma> {
    inlets
        .iter()
        .enumerate()
        .map(|(i, inlet)| {
            let name = format!("inlet{}", i);
            let by_ref = None;
            let mutability = None;
            let ident = syn::Ident::new(&name, proc_macro2::Span::call_site());
            let subpat = None;
            let pat_ident = syn::PatIdent {
                by_ref,
                mutability,
                ident,
                subpat,
            };
            let pat = pat_ident.into();
            let colon_token = Default::default();
            let ty = inlet.ty.clone();
            let arg_captured = syn::ArgCaptured {
                pat,
                colon_token,
                ty,
            };
            syn::FnArg::from(arg_captured)
        })
        .collect()
}

fn graph_node_evaluator_fn_output<Id>(outlets: &[Outlet<Id>]) -> syn::ReturnType {
    match outlets.len() {
        0 => syn::ReturnType::Default,
        1 => {
            let r_arrow = Default::default();
            let ty = Box::new(outlets[0].ty.clone());
            syn::ReturnType::Type(r_arrow, ty)
        }
        _ => {
            let paren_token = Default::default();
            let elems = outlets.iter().map(|outlet| outlet.ty.clone()).collect();
            let ty_tuple = syn::TypeTuple { paren_token, elems };
            let r_arrow = Default::default();
            let ty = Box::new(syn::Type::Tuple(ty_tuple));
            syn::ReturnType::Type(r_arrow, ty)
        }
    }
}

pub mod codegen {
    use super::{Edge, Inlet, Outlet};
    use crate::node::{self, Node};
    use petgraph::visit::{
        Data, EdgeRef, GraphRef, IntoEdgesDirected, IntoNodeReferences, NodeIndexable, NodeRef,
        Visitable, Walker,
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

    /// Given a graph of gantz nodes, produce the `Evaluator` associated with each.
    pub fn node_evaluators<G>(g: G) -> NodeEvaluatorMap<G::NodeId>
    where
        G: IntoNodeReferences,
        <G::NodeRef as NodeRef>::Weight: Node,
        G::NodeId: Eq + Hash,
    {
        g.node_references()
            .map(|n| (n.id(), n.weight().evaluator()))
            .collect()
    }

    /// Given a set of node evaluators, return only those that have function definitions.
    pub fn node_evaluator_fns<Id>(
        evaluators: &NodeEvaluatorMap<Id>,
    ) -> impl Iterator<Item = (&Id, &syn::ItemFn)>
    where
        Id: Eq + Hash,
    {
        evaluators.iter().filter_map(|(id, eval)| match eval {
            node::Evaluator::Fn { ref fn_item } => Some((id, fn_item)),
            node::Evaluator::Expr { .. } => None,
        })
    }

    /// Given a graph of gantz nodes, return `NodeId`s of those that require push evaluation.
    ///
    /// Expects any graph type whose nodes implement `Node`.
    pub fn push_nodes<G>(g: G) -> Vec<(G::NodeId, node::EvalFn)>
    where
        G: IntoNodeReferences,
        <G::NodeRef as NodeRef>::Weight: Node,
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
        <G::NodeRef as NodeRef>::Weight: Node,
    {
        g.node_references()
            .filter_map(|n| n.weight().pull_eval().map(|eval| (n.id(), eval)))
            .collect()
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
        // First, find all nodes reachable by a `DFS` from this node.
        let dfs: HashSet<G::NodeId> = petgraph::visit::Dfs::new(g, n).iter(g).collect();

        // The order of evaluation is topological order of nodes touching the DFS.
        petgraph::visit::Topo::new(g)
            .iter(g)
            .filter(move |node| dfs.contains(&node))
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
        // First, find all nodes reachable by a `DFS` from this node.
        let rev_g = petgraph::visit::Reversed(&g);
        let dfs: HashSet<G::NodeId> = petgraph::visit::Dfs::new(rev_g, n).iter(rev_g).collect();

        // The order of evaluation is topological order of nodes touching the reverse DFS.
        petgraph::visit::Topo::new(g)
            .iter(g)
            .filter(move |node| dfs.contains(&node))
    }

    /// Given a node evaluation order, produce the series of evaluation steps required.
    pub fn eval_steps<G, I>(
        g: G,
        node_evaluators: &NodeEvaluatorMap<G::NodeId>,
        eval_order: I,
    ) -> Vec<EvalStep<G::NodeId>>
    where
        G: GraphRef + IntoEdgesDirected + IntoNodeReferences + NodeIndexable,
        G: Data<EdgeWeight = Edge>,
        G::NodeId: Eq + Hash,
        <G::NodeRef as NodeRef>::Weight: Node,
        I: IntoIterator<Item = G::NodeId>,
    {
        let mut eval_steps = vec![];

        // Step through each of the nodes.
        for node in eval_order {
            // Initialise the arguments to `None` for each input.
            let child_evaluator = &node_evaluators[&node];
            let mut args: Vec<_> = (0..child_evaluator.n_inputs()).map(|_| None).collect();

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

    /// Given a function argument, return its type if known.
    pub fn ty_from_fn_arg(arg: &syn::FnArg) -> Option<syn::Type> {
        match arg {
            syn::FnArg::Captured(cap) => Some(cap.ty.clone()),
            syn::FnArg::Ignored(ty) => Some(ty.clone()),
            _ => None,
        }
    }

    /// Generate a function for performing evaluation of the given steps.
    pub fn eval_fn<G>(
        g: G,
        eval_fn: node::EvalFn,
        steps: &[EvalStep<G::NodeId>],
        node_evaluators: &NodeEvaluatorMap<G::NodeId>,
    ) -> syn::ItemFn
    where
        G: GraphRef + IntoNodeReferences + NodeIndexable,
        G::NodeId: Eq + Hash,
        <G::NodeRef as NodeRef>::Weight: Node,
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

        // For each evaluation step, generate a statement where the expression for the node at that
        // evaluation step is evaluated and the outputs are destructured from a tuple.
        let mut stmts: Vec<syn::Stmt> = vec![];

        // Keep track of each of the lvalues for each of the statements. These are used to pass
        let mut lvalues: HashMap<(G::NodeId, node::Output), syn::Ident> = Default::default();

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
            let expr: syn::Expr = ne.expr(args);

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

            let stmt: syn::Stmt = syn::parse_quote! {
                let #lvals = #expr;
            };

            stmts.push(stmt);
        }

        // Construct the final function item.
        let block = Box::new(syn::Block {
            stmts,
            brace_token: Default::default(),
        });
        let node::EvalFn {
            fn_decl,
            fn_name,
            mut fn_attrs,
        } = eval_fn;
        let decl = Box::new(fn_decl);
        let ident = syn::Ident::new(&fn_name, proc_macro2::Span::call_site());
        let vis = syn::Visibility::Public(syn::VisPublic {
            pub_token: Default::default(),
        });

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
    pub fn eval_fns<'a, G, I>(
        g: G,
        eval_nodes: I,
        node_evaluators: &NodeEvaluatorMap<G::NodeId>,
    ) -> Vec<syn::ItemFn>
    where
        G: GraphRef + IntoNodeReferences + NodeIndexable,
        G::NodeId: 'a + Eq + Hash,
        <G::NodeRef as NodeRef>::Weight: Node,
        I: IntoIterator<Item = (G::NodeId, node::EvalFn, &'a [EvalStep<G::NodeId>])>,
    {
        eval_nodes
            .into_iter()
            .map(|(_n, eval, steps)| eval_fn(g, eval, steps, node_evaluators))
            .collect()
    }

    /// Given a gantz graph, generate the rust code src file with all the necessary functions for
    /// executing it.
    pub fn file<G>(g: G, _inlets: &[Inlet<G::NodeId>], _outlets: &[Outlet<G::NodeId>]) -> syn::File
    where
        G: GraphRef + IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable,
        G: Data<EdgeWeight = Edge>,
        G::NodeId: Eq + Hash,
        <G::NodeRef as NodeRef>::Weight: Node,
    {
        let node_evaluators = node_evaluators(g);
        let node_evaluator_fn_items = node_evaluator_fns(&node_evaluators);
        let pull_nodes = pull_nodes(g);
        let push_nodes = push_nodes(g);

        let pull_node_fn_items = pull_nodes.into_iter().map(|(n, eval)| {
            let order = pull_eval_order(g, n);
            let steps = eval_steps(g, &node_evaluators, order);
            let item_fn = eval_fn(g, eval, &steps, &node_evaluators);
            syn::Item::Fn(item_fn)
        });

        let push_node_fn_items = push_nodes.into_iter().map(|(n, eval)| {
            let order = push_eval_order(&g, n);
            let steps = eval_steps(g, &node_evaluators, order);
            let item_fn = eval_fn(g, eval, &steps, &node_evaluators);
            syn::Item::Fn(item_fn)
        });

        let items = node_evaluator_fn_items
            .map(|(_, item_fn)| syn::Item::Fn(item_fn.clone()))
            .chain(pull_node_fn_items)
            .chain(push_node_fn_items)
            .collect();

        let file = syn::File {
            shebang: None,
            attrs: vec![],
            items,
        };
        file
    }

    // Create the `#[no_mangle]` attribute.
    fn no_mangle_attr() -> syn::Attribute {
        let ident = syn::Ident::new("no_mangle", proc_macro2::Span::call_site());
        let arguments = syn::PathArguments::None;
        let segments = Some(syn::PathSegment { ident, arguments })
            .into_iter()
            .collect();
        let path = syn::Path {
            leading_colon: None,
            segments,
        };
        let style = syn::AttrStyle::Outer;
        let pound_token = Default::default();
        let bracket_token = Default::default();
        let tts = Default::default();
        syn::Attribute {
            pound_token,
            style,
            bracket_token,
            path,
            tts,
        }
    }
}
