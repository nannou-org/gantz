use crate::env::Environment;
use dyn_clone::DynClone;
use dyn_hash::DynHash;
use std::any::Any;

/// A top-level blanket trait providing trait object cloning, hashing, and serialization.
#[typetag::serde(tag = "type")]
pub trait Node:
    Any
    + DynClone
    + DynHash
    + gantz_ca::CaHash
    + gantz_core::Node<Environment>
    + gantz_egui::NodeUi<Environment>
    + Send
    + Sync
{
}

dyn_clone::clone_trait_object!(Node);
dyn_hash::hash_trait_object!(Node);

#[typetag::serde]
impl Node for gantz_core::node::Expr {}
#[typetag::serde]
impl Node for gantz_core::node::GraphNode<Box<dyn Node>> {}
#[typetag::serde]
impl Node for gantz_core::node::graph::Inlet {}
#[typetag::serde]
impl Node for gantz_core::node::graph::Outlet {}

#[typetag::serde]
impl Node for gantz_std::ops::Add {}
#[typetag::serde]
impl Node for gantz_std::Bang {}
#[typetag::serde]
impl Node for gantz_std::Log {}
#[typetag::serde]
impl Node for gantz_std::Number {}

#[typetag::serde]
impl Node for gantz_egui::node::NamedGraph {}

#[typetag::serde]
impl Node for Box<dyn Node> {}

// To allow for navigating between nested graphs in a graph scene, we need to be
// able to downcast a node to a graph node.
impl gantz_egui::widget::graph_scene::ToGraphMut for Box<dyn Node> {
    type Node = Self;
    fn to_graph_mut(&mut self) -> Option<&mut gantz_core::node::graph::Graph<Self::Node>> {
        ((&mut **self) as &mut dyn Any)
            .downcast_mut::<gantz_core::node::GraphNode<Self::Node>>()
            .map(|node| &mut node.graph)
    }
}

