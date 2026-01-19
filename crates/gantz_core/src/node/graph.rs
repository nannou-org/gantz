//! An implementation of a node acting as a nested graph.

use crate::{
    Edge, GRAPH_STATE,
    node::{self, Node},
    visit,
};
use gantz_ca::CaHash;
use petgraph::{
    Directed,
    graph::{EdgeIndex, NodeIndex},
    visit::{
        Data, IntoEdgeReferences, IntoEdgesDirected, IntoNodeReferences, NodeIndexable, NodeRef,
        Visitable,
    },
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    hash::{Hash, Hasher},
    ops::{Deref, DerefMut},
};
use steel::{SteelVal, parser::ast::ExprKind, steel_vm::engine::Engine};

/// The graph type used by the graph node to represent its nested graph.
pub type Graph<N> = petgraph::stable_graph::StableGraph<N, Edge, Directed, Index>;

/// The type used for indexing into the graph.
pub type Index = usize;
/// The type used to index into a graph's node's.
pub type NodeIx = NodeIndex<Index>;
/// The type used to index into a graph's edge's.
pub type EdgeIx = EdgeIndex<Index>;

/// A node that itself is implemented in terms of a graph of nodes.
#[derive(Clone, Debug)]
pub struct GraphNode<N> {
    /// The nested graph.
    pub graph: Graph<N>,
}

/// An inlet to a nested graph.
///
/// Inlet values are provided via `define` bindings by the parent graph node.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Inlet;

/// An outlet from a nested graph.
///
/// Outlet values are passed through directly as the node's output.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Outlet;

impl<N> Default for GraphNode<N> {
    fn default() -> Self {
        let graph = Default::default();
        GraphNode { graph }
    }
}

impl<N> Hash for GraphNode<N>
where
    N: Hash,
{
    fn hash<H>(&self, hasher: &mut H)
    where
        H: Hasher,
    {
        crate::graph::hash(&self.graph, hasher);
    }
}

impl<Env, N> Node<Env> for GraphNode<N>
where
    N: Node<Env>,
{
    fn n_inputs(&self, _: &Env) -> usize {
        inlets(&self.graph).count()
    }

    fn n_outputs(&self, _: &Env) -> usize {
        outlets(&self.graph).count()
    }

    fn branches(&self, _: &Env) -> Vec<node::EvalConf> {
        // TODO: generate branches based on inner node branching
        vec![]
    }

    fn expr(&self, ctx: node::ExprCtx<Env>) -> ExprKind {
        nested_expr(ctx.env(), &self.graph, ctx.path(), ctx.inputs())
    }

    fn stateful(&self) -> bool {
        true
    }

    fn register(&self, path: &[node::Id], vm: &mut Engine) {
        // Register the graph's state map.
        node::state::update_value(vm, path, SteelVal::empty_hashmap())
            .expect("failed to register graph hashmap");
    }

    fn visit(&self, ctx: visit::Ctx<Env>, visitor: &mut dyn node::Visitor<Env>) {
        crate::graph::visit(ctx.env(), &self.graph, ctx.path(), visitor);
    }
}

impl<N: PartialEq> PartialEq for GraphNode<N> {
    fn eq(&self, other: &Self) -> bool {
        graph_partial_eq(self, other)
    }
}

impl<N: Eq> Eq for GraphNode<N> {}

// Manual implementation of `Deserialize` as it cannot be derived for a struct with associated
// types without unnecessary trait bounds on the struct itself.
impl<'de, N> Deserialize<'de> for GraphNode<N>
where
    N: Deserialize<'de>,
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
        }

        struct GraphNodeVisitor<G>(std::marker::PhantomData<G>);

        impl<'de, N> Visitor<'de> for GraphNodeVisitor<Graph<N>>
        where
            N: Deserialize<'de>,
        {
            type Value = GraphNode<N>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("struct GraphNode")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<GraphNode<N>, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let graph = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                Ok(GraphNode { graph })
            }

            fn visit_map<V>(self, mut map: V) -> Result<GraphNode<N>, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut graph = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Graph => {
                            if graph.is_some() {
                                return Err(de::Error::duplicate_field("graph"));
                            }
                            graph = Some(map.next_value()?);
                        }
                    }
                }
                let graph = graph.ok_or_else(|| de::Error::missing_field("graph"))?;
                Ok(GraphNode { graph })
            }
        }

        const FIELDS: &[&str] = &["graph"];
        let visitor: GraphNodeVisitor<Graph<N>> = GraphNodeVisitor(std::marker::PhantomData);
        deserializer.deserialize_struct("GraphNode", FIELDS, visitor)
    }
}

// Manual implementation of `Serialize` as it cannot be derived for a struct with associated
// types without unnecessary trait bounds on the struct itself.
impl<N> Serialize for GraphNode<N>
where
    N: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("GraphNode", 3)?;
        state.serialize_field("graph", &self.graph)?;
        state.end()
    }
}

impl<Env> Node<Env> for Inlet {
    /// Returns the inlet binding variable created by the parent graph node.
    fn expr(&self, ctx: node::ExprCtx<Env>) -> ExprKind {
        let node_ix = ctx.path().last().expect("inlet must have a path");
        let var_name = format!("inlet-{node_ix}");
        Engine::emit_ast(&var_name)
            .expect("failed to emit AST")
            .into_iter()
            .next()
            .unwrap()
            .into()
    }

    fn n_inputs(&self, _env: &Env) -> usize {
        0
    }

    fn n_outputs(&self, _env: &Env) -> usize {
        1
    }

    fn inlet(&self) -> bool {
        true
    }
}

impl<Env> Node<Env> for Outlet {
    /// Passes through the input value directly.
    fn expr(&self, ctx: node::ExprCtx<Env>) -> ExprKind {
        let input = match &ctx.inputs()[0] {
            Some(expr) => expr.clone(),
            None => "'()".to_string(),
        };
        Engine::emit_ast(&input)
            .expect("failed to emit AST")
            .into_iter()
            .next()
            .unwrap()
            .into()
    }

    fn n_inputs(&self, _env: &Env) -> usize {
        1
    }

    fn n_outputs(&self, _env: &Env) -> usize {
        0
    }

    fn outlet(&self) -> bool {
        true
    }
}

impl<N> CaHash for GraphNode<N>
where
    N: CaHash,
{
    fn hash(&self, hasher: &mut gantz_ca::Hasher) {
        gantz_ca::hash_graph(&self.graph, hasher);
    }
}

impl CaHash for Inlet {
    fn hash(&self, hasher: &mut gantz_ca::Hasher) {
        CaHash::hash("in", hasher);
    }
}

impl CaHash for Outlet {
    fn hash(&self, hasher: &mut gantz_ca::Hasher) {
        CaHash::hash("out", hasher);
    }
}

impl<N> Deref for GraphNode<N> {
    type Target = Graph<N>;
    fn deref(&self) -> &Self::Target {
        &self.graph
    }
}

impl<N> DerefMut for GraphNode<N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.graph
    }
}

/// A `PartialEq` implementation for [`Graph`].
pub fn graph_partial_eq<N: PartialEq>(a: &Graph<N>, b: &Graph<N>) -> bool {
    a.node_references()
        .zip(b.node_references())
        .all(|(a, b)| a == b)
        && a.edge_references()
            .zip(b.edge_references())
            .all(|(a, b)| a == b)
}

/// Count the number of inlet nodes in the given graph.
pub fn inlets<Env, G>(g: G) -> impl Iterator<Item = G::NodeRef>
where
    G: Data + IntoNodeReferences,
    G::NodeWeight: Node<Env>,
{
    g.node_references().filter(|n_ref| n_ref.weight().inlet())
}

/// Count the number of outlet nodes in the given graph.
pub fn outlets<Env, G>(g: G) -> impl Iterator<Item = G::NodeRef>
where
    G: Data + IntoNodeReferences,
    G::NodeWeight: Node<Env>,
{
    g.node_references().filter(|n_ref| n_ref.weight().outlet())
}

/// The implementation of the `GraphNode`'s `Node::expr` fn.
pub fn nested_expr<Env, G>(
    env: &Env,
    g: G,
    path: &[node::Id],
    inputs: &[Option<String>],
) -> ExprKind
where
    G: IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable + Data<EdgeWeight = Edge>,
    G::NodeWeight: Node<Env>,
    G::NodeId: Eq + Hash,
{
    use crate::compile;
    use petgraph::visit::EdgeRef;

    // Create define bindings for inlet values.
    let inlets: Vec<_> = inlets(g).map(|n_ref| n_ref.id()).collect();
    let mut inlet_bindings = Vec::new();
    for (i, &inlet_id) in inlets.iter().enumerate() {
        let node_ix = g.to_index(inlet_id);
        let input_expr = if i < inputs.len() && inputs[i].is_some() {
            inputs[i].as_ref().unwrap().clone()
        } else {
            "'()".to_string()
        };
        let binding = format!("(define inlet-{node_ix} {input_expr})");
        inlet_bindings.push(binding);
    }

    // Use compile to create the evaluation order, steps, and statements
    let meta = compile::Meta::from_graph(env, g);
    let outlets: Vec<_> = outlets(g).map(|n_ref| n_ref.id()).collect();
    let flow_graph = compile::flow_graph(
        &meta,
        inlets
            .iter()
            .map(|&n| (g.to_index(n), node::Conns::connected(1).unwrap())),
        outlets
            .iter()
            .map(|&n| (g.to_index(n), node::Conns::connected(1).unwrap())),
    );
    let stmts = compile::eval_fn_body(
        path,
        &meta.graph,
        &meta.stateful,
        &meta.inlets,
        &meta.outlets,
        &flow_graph,
    );

    // Combine inlet bindings with graph evaluation steps
    let all_stmts = inlet_bindings
        .into_iter()
        .chain(stmts.iter().map(|stmt| format!("{}", stmt)))
        .collect::<Vec<_>>()
        .join(" ");

    // For the return values, find the source node connected to each outlet's input.
    let outlet_values = outlets
        .iter()
        .map(|&outlet_id| {
            // Find the edge coming into this outlet (input index 0).
            let incoming: Vec<_> = g.edges_directed(outlet_id, petgraph::Incoming).collect();
            if let Some(edge_ref) = incoming.first() {
                let src_ix = g.to_index(edge_ref.source());
                let src_out = edge_ref.weight().output.0;
                format!("node-{src_ix}-o{src_out}")
            } else {
                // No incoming edge, outlet is disconnected.
                "'()".to_string()
            }
        })
        .collect::<Vec<_>>();

    // Construct the final expression based on number of outputs
    let outlet_values_expr = if outlet_values.len() > 1 {
        let ret_values = outlet_values.join(" ");
        format!("(values {})", ret_values)
    } else if outlet_values.len() == 1 {
        outlet_values[0].clone()
    } else {
        "'()".to_string()
    };

    let expr_str = format!(
        "(begin (define {GRAPH_STATE} state)
           {}
           (set! state {GRAPH_STATE})
           {outlet_values_expr})",
        all_stmts
    );
    Engine::emit_ast(&expr_str)
        .expect("failed to emit AST for nested expr")
        .into_iter()
        .next()
        .unwrap()
        .into()
}
