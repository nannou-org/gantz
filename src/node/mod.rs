use petgraph::graph::NodeIndex;
use std::any::Any;
use std::ops;

pub mod add;

/// Implemented for nodes, representing the state.
pub trait State: Any + 'static {
    /// The number of outlets on the node.
    fn n_outlets(&self) -> u32;

    /// The number of inlets on the node.
    fn n_inlets(&self) -> u32;

    /// Process the given incoming data for the inlet.
    fn proc_inlet_at_index(&mut self, Inlet, incoming: &Any);

    /// Process and prepare the outlet for the outlet at the given index.
    fn proc_outlet_at_index(&mut self, Outlet) -> &Any;

    /// A reference to the current outlet value.
    fn outlet_ref(&self, Outlet) -> &Any;
}

/// The type used to represent the index of an inlet or outlet.
pub type ConnectionIndex = u32;

/// Represents a specific inlet of a node.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Inlet(pub ConnectionIndex);

/// Represents a specific outlet of a node.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Outlet(pub ConnectionIndex);

/// The index type used to uniquely identify a single **Node**.
pub type Index = NodeIndex<super::Index>;

/// The **Node** type stored within the **Graph**.
pub struct Node {
    pub state: Box<State>,
}

/// The container for the node encapsulating space for the inlets, outlet and the node itself.
pub struct Container<I, N, O> {
    pub inlets: I,
    pub node: N,
    pub outlet: Option<O>,
}

impl State for Box<State> {
    fn n_outlets(&self) -> u32 {
        (**self).n_outlets()
    }
    fn n_inlets(&self) -> u32 {
        (**self).n_inlets()
    }
    fn proc_inlet_at_index(&mut self, i: Inlet, incoming: &Any) {
        (**self).proc_inlet_at_index(i, incoming)
    }
    fn proc_outlet_at_index(&mut self, i: Outlet) -> &Any {
        (**self).proc_outlet_at_index(i)
    }
    fn outlet_ref(&self, i: Outlet) -> &Any {
        (**self).outlet_ref(i)
    }
}

impl ops::Deref for Node {
    type Target = State;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl ops::DerefMut for Node {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}
