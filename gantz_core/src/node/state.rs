use super::{Deserialize, Serialize};
use crate::node::{self, Node};

/// A trait implemented for all **Node** types allowing to add some state accessible to its
/// expression. This is particularly useful for adding state to **Expr** nodes.
pub trait WithStateType: Sized + Node {
    /// Consume `self` and return a `Node` that has state of type `state_type`.
    fn with_state_type(self, state_type: syn::Type) -> State<Self>;

    /// A short-hand for `with_state_type` - allows for describing the type via a `str`.
    fn with_state_ty(self, state_type: &str) -> syn::Result<State<Self>> {
        let ty: syn::Type = syn::parse_str(state_type)?;
        Ok(self.with_state_type(ty))
    }
}

/// A wrapper around a **Node** that adds some persistent state.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct State<N> {
    pub node: N,
    /// Attributes for the generated `ItemFn`.
    #[serde(with = "crate::node::serde::ty")]
    pub state_type: syn::Type,
}

impl<N> State<N> {
    /// Given some node, return a **State** node enabling access to state of the given type.
    pub fn new(node: N, state_type: syn::Type) -> Self {
        State { node, state_type }
    }
}

impl<N> WithStateType for N
where
    N: Node,
{
    fn with_state_type(self, state_type: syn::Type) -> State<Self> {
        State::new(self, state_type)
    }
}

impl<N> Node for State<N>
where
    N: Node,
{
    fn evaluator(&self) -> node::Evaluator {
        self.node.evaluator()
    }

    fn push_eval(&self) -> Option<node::EvalFn> {
        self.node.push_eval()
    }

    fn pull_eval(&self) -> Option<node::EvalFn> {
        self.node.pull_eval()
    }

    fn state_type(&self) -> Option<syn::Type> {
        Some(self.state_type.clone())
    }

    fn crate_deps(&self) -> Vec<node::CrateDep> {
        self.node.crate_deps()
    }
}
