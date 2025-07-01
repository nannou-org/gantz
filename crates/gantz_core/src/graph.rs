use crate::{
    Edge, GRAPH_STATE,
    node::{self, Node},
    visit,
};
use petgraph::visit::{
    Data, EdgeRef, GraphBase, IntoEdgeReferences, IntoEdgesDirected, IntoNodeReferences,
    NodeIndexable, NodeRef, Visitable,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    hash::{Hash, Hasher},
    ops::{Deref, DerefMut},
};
use steel::{SteelVal, parser::ast::ExprKind, steel_vm::engine::Engine};

/// Required by graphs that support nesting graphs of the same type as nodes.
///
/// Used by the `GraphNode`'s `Node::expr` implementation.
pub trait NestedExpr: GraphBase {
    /// The expression used to evaluate a nested graph from its inputs to its
    /// outputs.
    fn nested_expr(
        &self,
        inlets: &[Self::NodeId],
        outlets: &[Self::NodeId],
        inputs: &[Option<ExprKind>],
    ) -> ExprKind;
}

/// A trait implemented for graph types capable of adding nodes and returning a
/// unique ID associated with the added node.
///
/// This trait allows for providing the `GraphNode::add_inlet` and
/// `add_outlet` methods.
pub trait AddNode: Data {
    /// Add a node with the given weight and return its unique ID.
    fn add_node(&mut self, n: Self::NodeWeight) -> Self::NodeId;
}

/// The name of the function generated for performing full evaluation of the
/// graph.
pub const FULL_EVAL_FN_NAME: &str = "full_eval";

/// A node that itself is implemented in terms of a graph of nodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphNode<G>
where
    G: GraphBase,
{
    /// The graph used to evaluate the node.
    pub graph: G,
    /// The types of each of the inputs into the graph node.
    pub inlets: Vec<G::NodeId>,
    /// The types of each of the outputs into the graph node.
    pub outlets: Vec<G::NodeId>,
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

impl<G> GraphNode<G>
where
    G: AddNode,
{
    /// Adds the given `NodeId` to the graph as an inlet node.
    ///
    /// This is the same as `G::add_node`, but also adds the resulting node index to the
    /// `GraphNode`'s `inlets` list.
    pub fn add_inlet(&mut self, n: G::NodeWeight) -> G::NodeId {
        let id = self.add_node(n);
        self.inlets.push(id);
        id
    }

    /// Adds the given `NodeId` to the graph as an outlet node.
    ///
    /// This is the same as `G::add_node`, but also adds the resulting node index to the
    /// `GraphNode`'s `outlet` list.
    pub fn add_outlet(&mut self, n: G::NodeWeight) -> G::NodeId {
        let id = self.add_node(n);
        self.outlets.push(id);
        id
    }
}

impl<'a, G> AddNode for &'a mut G
where
    G: AddNode,
{
    fn add_node(&mut self, n: Self::NodeWeight) -> Self::NodeId {
        (**self).add_node(n)
    }
}

impl<G> NestedExpr for G
where
    G: IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable + Data<EdgeWeight = Edge>,
    G::NodeWeight: Node,
    G::NodeId: Eq + Hash,
{
    fn nested_expr(
        &self,
        inlets: &[Self::NodeId],
        outlets: &[Self::NodeId],
        inputs: &[Option<ExprKind>],
    ) -> ExprKind {
        use crate::codegen;

        // Create statements to set inlet node states from inputs
        let mut inlet_bindings = Vec::new();
        for (i, &inlet_id) in inlets.iter().enumerate() {
            if i < inputs.len() && inputs[i].is_some() {
                let input_expr = inputs[i].as_ref().unwrap();
                let node_ix = self.to_index(inlet_id);
                let binding = format!(
                    "(set! {GRAPH_STATE} \
                       (hash-insert {GRAPH_STATE} '{node_ix} {input_expr}))",
                );
                inlet_bindings.push(binding);
            }
        }

        // Use codegen to create the evaluation order, steps, and statements
        let order = codegen::eval_order(self, inlets.iter().cloned(), outlets.iter().cloned());
        let steps = codegen::eval_steps(self, order);
        // FIXME: path
        let stmts = codegen::eval_stmts(self, &[], &steps);

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
                let node_ix = self.to_index(outlet_id);
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
}

impl<G> Default for GraphNode<G>
where
    G: GraphBase + Default,
{
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

impl<G> Hash for GraphNode<G>
where
    G: Data,
    for<'a> &'a G: Data<EdgeWeight = G::EdgeWeight, NodeWeight = G::NodeWeight>
        + GraphBase<EdgeId = G::EdgeId, NodeId = G::NodeId>
        + IntoEdgeReferences
        + IntoNodeReferences,
    G::EdgeId: Hash,
    G::EdgeWeight: Hash,
    G::NodeId: Hash,
    G::NodeWeight: Hash,
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

impl<G> Node for GraphNode<G>
where
    G: Data + NodeIndexable,
    G::NodeWeight: Node,
    for<'a> &'a G: Data<NodeWeight = G::NodeWeight>
        + GraphBase<NodeId = G::NodeId>
        + IntoNodeReferences
        + NestedExpr,
{
    fn expr(&self, inputs: &[Option<ExprKind>]) -> ExprKind {
        let g: &G = &self.graph;
        g.nested_expr(&self.inlets, &self.outlets, inputs)
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

        // Register each of the child nodes.
        let mut path = path.to_vec();
        for n in self.graph.node_references() {
            let id = self.graph.to_index(n.id());
            path.push(id);
            n.weight().register(&path, vm);
            path.pop();
        }
    }

    fn visit(&self, visitor: &mut dyn node::Visitor, path: &[node::Id]) {
        visitor.visit_pre(self, path);
        visit(&self.graph, path, visitor);
        visitor.visit_post(self, path);
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

impl Node for Inlet {
    /// Simply returns the state value as this node's output
    fn expr(&self, _inputs: &[Option<ExprKind>]) -> ExprKind {
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

    fn stateful(&self) -> bool {
        true
    }

    fn register(&self, path: &[node::Id], vm: &mut Engine) {
        node::state::update_value(vm, path, steel::SteelVal::Void).unwrap();
    }
}

impl Node for Outlet {
    // Stores the input value in the state.
    fn expr(&self, inputs: &[Option<ExprKind>]) -> ExprKind {
        let input = match &inputs[0] {
            Some(expr) => expr.to_string(),
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

    fn stateful(&self) -> bool {
        true
    }

    fn register(&self, path: &[node::Id], vm: &mut Engine) {
        node::state::update_value(vm, path, steel::SteelVal::Void).unwrap();
    }
}

impl<N, E, Ty, Ix> AddNode for petgraph::Graph<N, E, Ty, Ix>
where
    Ty: petgraph::EdgeType,
    Ix: petgraph::graph::IndexType,
{
    fn add_node(&mut self, n: N) -> petgraph::graph::NodeIndex<Ix> {
        petgraph::Graph::add_node(self, n)
    }
}

impl<N, E, Ty, Ix> AddNode for petgraph::stable_graph::StableGraph<N, E, Ty, Ix>
where
    Ty: petgraph::EdgeType,
    Ix: petgraph::graph::IndexType,
{
    fn add_node(&mut self, n: N) -> petgraph::graph::NodeIndex<Ix> {
        petgraph::stable_graph::StableGraph::add_node(self, n)
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

/// Visit all nodes in the graph, and all nested nodes in depth-first order.
pub fn visit<G>(graph: G, path: &[node::Id], visitor: &mut dyn node::Visitor)
where
    G: IntoNodeReferences + NodeIndexable,
    G::NodeWeight: Node,
{
    let mut path = path.to_vec();
    for n in graph.node_references() {
        let id = graph.to_index(n.id());
        path.push(id);
        node::visit(n.weight(), &path, visitor);
        path.pop();
    }
}

/// Register the given graph of nodes, including any nested nodes.
pub fn register<G>(graph: G, path: &[node::Id], vm: &mut Engine)
where
    G: IntoNodeReferences + NodeIndexable,
    G::NodeWeight: Node,
{
    visit(graph, path, &mut visit::Register(vm));
}
