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
        NodeRef, Topo, Visitable,
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
    /// The types of each of the inputs into the graph node.
    pub inlets: Vec<NodeIx>,
    /// The types of each of the outputs into the graph node.
    pub outlets: Vec<NodeIx>,
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

impl<N> GraphNode<N> {
    /// Adds the given `NodeId` to the graph as an inlet node.
    ///
    /// This is the same as `G::add_node`, but also adds the resulting node index to the
    /// `GraphNode`'s `inlets` list.
    pub fn add_inlet(&mut self, n: N) -> NodeIx {
        let id = self.graph.add_node(n);
        self.inlets.push(id);
        id
    }

    /// Adds the given `NodeId` to the graph as an outlet node.
    ///
    /// This is the same as `G::add_node`, but also adds the resulting node index to the
    /// `GraphNode`'s `outlet` list.
    pub fn add_outlet(&mut self, n: N) -> NodeIx {
        let id = self.graph.add_node(n);
        self.outlets.push(id);
        id
    }
}

impl<N> Default for GraphNode<N> {
    fn default() -> Self {
        let graph = Default::default();
        let inlets = Default::default();
        let outlets = Default::default();
        GraphNode {
            graph,
            inlets,
            outlets,
        }
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
    fn expr(&self, ctx: node::ExprCtx) -> ExprKind {
        nested_expr(
            &self.graph,
            ctx.path(),
            &self.inlets,
            &self.outlets,
            ctx.inputs(),
        )
    }

    fn n_inputs(&self) -> usize {
        self.inlets.len()
    }

    fn n_outputs(&self) -> usize {
        self.outlets.len()
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
        visit(&self.graph, ctx.path(), visitor);
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
            Inlets,
            Outlets,
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

            fn visit_map<V>(self, mut map: V) -> Result<GraphNode<N>, V::Error>
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
        state.serialize_field("inlets", &self.inlets)?;
        state.serialize_field("outlets", &self.outlets)?;
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

/// The implementation of the `GraphNode`'s `Node::expr` fn.
fn nested_expr<G>(
    g: G,
    path: &[node::Id],
    inlets: &[G::NodeId],
    outlets: &[G::NodeId],
    inputs: &[Option<String>],
) -> ExprKind
where
    G: IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable + Data<EdgeWeight = Edge>,
    G::NodeWeight: Node,
    G::NodeId: Eq + Hash,
{
    use crate::codegen;

    // Create statements to set inlet node states from inputs
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
    let flow = codegen::Flow::from_graph(g);
    let order = codegen::eval_order(g, inlets.iter().cloned(), outlets.iter().cloned())
        .map(|id| g.to_index(id));
    let steps: Vec<_> = codegen::eval_steps(&flow, order).collect();
    let stmts = codegen::eval_stmts(path, &steps, &flow.outputs, &flow.stateful);

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
        "(begin (define __graph_state state) {} {outlet_values_expr})",
        all_stmts
    );
    Engine::emit_ast(&expr_str)
        .expect("failed to emit AST for nested expr")
        .into_iter()
        .next()
        .unwrap()
}

// --------------------------------------------------------

/// Visit all nodes in the graph in toposort order, and all nested nodes in
/// depth-first order.
pub fn visit<G>(g: G, path: &[node::Id], visitor: &mut dyn node::Visitor)
where
    G: Data<EdgeWeight = Edge> + IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable,
    G::NodeWeight: Node,
{
    let mut path = path.to_vec();
    let mut topo = Topo::new(g);
    while let Some(n) = topo.next(g) {
        let ix = g.to_index(n);
        path.push(ix);
        let inputs: Vec<_> = g
            .edges_directed(n, petgraph::Direction::Incoming)
            .map(|e_ref| (g.to_index(e_ref.source()), e_ref.weight().clone()))
            .collect();
        let ctx = visit::Ctx::new(&path, &inputs);

        // FIXME: index directly.
        let nref = g.node_references().find(|nref| nref.id() == n).unwrap();

        node::visit(ctx, nref.weight(), visitor);
        path.pop();
    }
}

/// Register the given graph of nodes, including any nested nodes.
pub fn register<G>(g: G, path: &[node::Id], vm: &mut Engine)
where
    G: Data<EdgeWeight = Edge> + IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable,
    G::NodeWeight: Node,
{
    visit(g, path, &mut visit::Register(vm));
}
