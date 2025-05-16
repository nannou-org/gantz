use crate::node::{self, Node};
pub use serde::{Deserialize, Serialize};

/// A wrapper around the **Node** trait that allows for serializing and
/// deserializing node trait objects.
#[typetag::serde(tag = "type")]
pub trait SerdeNode {
    fn node(&self) -> &dyn Node;
}

#[typetag::serde]
impl SerdeNode for node::Expr {
    fn node(&self) -> &dyn Node {
        self
    }
}

#[typetag::serde]
impl SerdeNode for node::Push<node::Expr> {
    fn node(&self) -> &dyn Node {
        self
    }
}

#[typetag::serde]
impl SerdeNode for node::Pull<node::Expr> {
    fn node(&self) -> &dyn Node {
        self
    }
}
