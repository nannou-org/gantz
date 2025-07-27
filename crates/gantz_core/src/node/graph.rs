//! An implementation of a node acting as a nested graph.

use crate::{
    Edge, GRAPH_STATE,
    node::{self, Node},
    visit,
};
use petgraph::{
    Directed,
    graph::{EdgeIndex, NodeIndex},
    visit::{
        Data, EdgeRef, IntoEdgeReferences, IntoEdgesDirected, IntoNodeReferences, NodeIndexable,
        NodeRef, Visitable,
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
/// Inlet values are received in the `Inlet`'s `state`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Inlet;

/// An outlet from a nested graph.
///
/// Outlet values are made available via the `Outlet`'s `state`
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
        for n in self.graph.node_references() {
            n.id().hash(hasher);
            n.weight().hash(hasher);
        }
        for e in self.graph.edge_references() {
            e.id().hash(hasher);
            e.weight().hash(hasher);
        }
    }
}

impl<N> Node for GraphNode<N>
where
    N: Node,
{
    fn branches(&self) -> Vec<node::EvalConf> {
        todo!("branch based on inner node branching")
    }

    fn expr(&self, ctx: node::ExprCtx) -> ExprKind {
        nested_expr(&self.graph, ctx.path(), ctx.inputs())
    }

    fn n_inputs(&self) -> usize {
        inlets(&self.graph).count()
    }

    fn n_outputs(&self) -> usize {
        outlets(&self.graph).count()
    }

    fn stateful(&self) -> bool {
        true
    }

    fn register(&self, path: &[node::Id], vm: &mut Engine) {
        // Register the graph's state map.
        node::state::update_value(vm, path, SteelVal::empty_hashmap())
            .expect("failed to register graph hashmap");
    }

    fn visit(&self, ctx: visit::Ctx, visitor: &mut dyn node::Visitor) {
        crate::graph::visit(&self.graph, ctx.path(), visitor);
    }
}

impl<N: PartialEq> PartialEq for GraphNode<N> {
    fn eq(&self, other: &Self) -> bool {
        self.graph
            .node_references()
            .zip(other.graph.node_references())
            .all(|(a, b)| a == b)
            && self
                .graph
                .edge_references()
                .zip(other.graph.edge_references())
                .all(|(a, b)| a == b)
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

impl Node for Inlet {
    /// Simply returns the state value as this node's output
    fn expr(&self, _ctx: node::ExprCtx) -> ExprKind {
        Engine::emit_ast("state")
            .expect("failed to emit AST")
            .into_iter()
            .next()
            .unwrap()
            .into()
    }

    fn n_inputs(&self) -> usize {
        0
    }

    fn n_outputs(&self) -> usize {
        1
    }

    fn inlet(&self) -> bool {
        true
    }

    fn stateful(&self) -> bool {
        true
    }

    fn register(&self, path: &[node::Id], vm: &mut Engine) {
        node::state::update_value(vm, path, steel::SteelVal::Void).unwrap();
    }
}

impl Node for Outlet {
    // Stores the input value in the state.
    fn expr(&self, ctx: node::ExprCtx) -> ExprKind {
        let input = match &ctx.inputs()[0] {
            Some(expr) => expr.clone(),
            None => "'()".to_string(),
        };
        let expr_str = format!("(begin (set! state {}) state)", input);
        Engine::emit_ast(&expr_str)
            .expect("failed to emit AST")
            .into_iter()
            .next()
            .unwrap()
            .into()
    }

    fn n_inputs(&self) -> usize {
        1
    }

    fn n_outputs(&self) -> usize {
        0
    }

    fn outlet(&self) -> bool {
        true
    }

    fn stateful(&self) -> bool {
        true
    }

    fn register(&self, path: &[node::Id], vm: &mut Engine) {
        node::state::update_value(vm, path, steel::SteelVal::Void).unwrap();
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

/// Count the number of inlet nodes in the given graph.
pub fn inlets<G>(g: G) -> impl Iterator<Item = G::NodeRef>
where
    G: Data + IntoNodeReferences,
    G::NodeWeight: Node,
{
    g.node_references().filter(|n_ref| n_ref.weight().inlet())
}

/// Count the number of outlet nodes in the given graph.
pub fn outlets<G>(g: G) -> impl Iterator<Item = G::NodeRef>
where
    G: Data + IntoNodeReferences,
    G::NodeWeight: Node,
{
    g.node_references().filter(|n_ref| n_ref.weight().outlet())
}

/// The implementation of the `GraphNode`'s `Node::expr` fn.
fn nested_expr<G>(g: G, path: &[node::Id], inputs: &[Option<String>]) -> ExprKind
where
    G: IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable + Data<EdgeWeight = Edge>,
    G::NodeWeight: Node,
    G::NodeId: Eq + Hash,
{
    use crate::codegen;

    // Create statements to set inlet node states from inputs
    let inlets: Vec<_> = inlets(g).map(|n_ref| n_ref.id()).collect();
    let mut inlet_bindings = Vec::new();
    for (i, &inlet_id) in inlets.iter().enumerate() {
        if i < inputs.len() && inputs[i].is_some() {
            let input_expr = inputs[i].as_ref().unwrap();
            let node_ix = g.to_index(inlet_id);
            let binding = format!(
                "(set! {GRAPH_STATE} \
                   (hash-insert {GRAPH_STATE} '{node_ix} {input_expr}))",
            );
            inlet_bindings.push(binding);
        }
    }

    // Use codegen to create the evaluation order, steps, and statements
    let meta = codegen::Meta::from_graph(g);
    let outlets: Vec<_> = outlets(g).map(|n_ref| n_ref.id()).collect();
    let order = codegen::eval_order(
        g,
        inlets
            .iter()
            .map(|&n| (n, node::Conns::connected(1).unwrap())),
        outlets
            .iter()
            .map(|&n| (n, node::Conns::connected(1).unwrap())),
    )
    .map(|id| g.to_index(id));
    let steps: Vec<_> = codegen::eval_steps(&meta, order).collect();
    let stmts = codegen::eval_stmts(path, &steps, &meta.stateful);

    // Combine inlet bindings with graph evaluation steps
    let all_stmts = inlet_bindings
        .into_iter()
        .chain(stmts.iter().map(|stmt| format!("{}", stmt)))
        .collect::<Vec<_>>()
        .join(" ");

    // For the return values, access the states of the outlet nodes
    let outlet_values = outlets
        .iter()
        .map(|&outlet_id| {
            let node_ix = g.to_index(outlet_id);
            format!("(hash-ref {GRAPH_STATE} '{node_ix})")
        })
        .collect::<Vec<_>>();

    // Construct the final expression based on number of outputs
    let outlet_values_expr = if outlet_values.len() > 1 {
        let ret_values = outlet_values.join(" ");
        format!("(values {})", ret_values)
    } else if outlet_values.len() == 1 {
        format!("{}", outlet_values[0])
    } else {
        format!("'()")
    };

    let expr_str = format!(
        "(begin (define __graph_state state)
           {}
           (set! state __graph_state)
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
