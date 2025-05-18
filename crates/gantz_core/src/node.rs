pub use expr::{Expr, ExprError};
pub use pull::{Pull, WithPullEval};
pub use push::{Push, WithPushEval};
pub use serde::SerdeNode;
use serde::{Deserialize, Serialize};
pub use state::{NodeState, State, WithStateType};
use steel::{parser::ast::ExprKind, steel_vm::engine::Engine};

pub mod expr;
pub mod pull;
pub mod push;
pub mod serde;
pub mod state;

pub trait Node {
    /// The number of inputs to the node.
    fn n_inputs(&self) -> usize {
        0
    }

    /// The number of outputs from the node.
    fn n_outputs(&self) -> usize {
        0
    }

    /// The expression that, given the expressions of connected inputs,
    /// produces the output(s).
    ///
    /// The given `inputs` slice is guaranteed to match the length of a call to
    /// [`Node::n_inputs`] immediately prior. Inputs are `Some` in the case that
    /// they are connected, and `None` otherwise.
    ///
    /// If [`Node::n_outputs`] is 1, the expr should result in a single value.
    ///
    /// If [`Node::n_outputs`] is > 1, the expr should result in a list of values.
    fn expr(&self, inputs: &[Option<ExprKind>]) -> ExprKind;

    /// Specifies whether or not code should be generated to allow for push
    /// evaluation from instances of this node. Enabling push evaluation allows
    /// applications to call into the graph by loading the resulting generated
    /// code at runtime.
    ///
    /// Push evaluation order is equivalent to a topological ordering of the
    /// connected component that starts from the `push_eval` node.
    ///
    /// Within a **Graph** node, a new function will be generated for each node
    /// that signals **Some**.  If **Some**, a function will be generated with
    /// the given **Signature** that represents pushing evaluation from this
    /// node.
    ///
    /// By default, this is **None**.
    fn push_eval(&self) -> Option<EvalFn> {
        None
    }

    /// Specifies whether or not code should be generated to allow for pull
    /// evaluation from instances of this node. Enabling pull evaluation allows
    /// applications to call into the graph by loading the resulting generated
    /// code at runtime.
    ///
    /// Pull evaluation order is equivalent to a topological ordering of the
    /// connected component that ends at the `pull_eval` node.
    ///
    /// Within a **Graph** node, a new function will be generated for each node
    /// that signals **Some**.  If **Some**, a function will be generated with
    /// the given **Signature** that represents pulling evaluation from this
    /// node.
    ///
    /// By default, this is **None**.
    fn pull_eval(&self) -> Option<EvalFn> {
        None
    }

    /// Whether or not the node requires access to state.
    ///
    /// Nodes returning `true` will have a special `state` variable accessible
    /// within their [`Node::expr`] provided during codegen.
    fn stateful(&self) -> bool {
        false
    }

    /// Function for registering necessary types, functions and initialising any
    /// default values as necessary.
    ///
    /// Nodes returning `true` from their [`Node::stateful`] implementation
    /// must use this to initialise their state.
    ///
    /// By default, the node is assumed to be stateless, and this does nothing.
    fn register(&self, _path: &[Id], _vm: &mut Engine) {}
}

/// Type used to represent a node's ID within a graph.
pub type Id = usize;

/// Represents a function that can be called to begin evaluation of the graph
/// from some node.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EvalFn;

/// Represents an input of a node via an index.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct Input(pub u16);

/// Represents an output of a node via an index.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct Output(pub u16);

impl<'a, N> Node for &'a N
where
    N: ?Sized + Node,
{
    fn n_inputs(&self) -> usize {
        (**self).n_inputs()
    }

    fn n_outputs(&self) -> usize {
        (**self).n_outputs()
    }

    fn expr(&self, inputs: &[Option<ExprKind>]) -> ExprKind {
        (**self).expr(inputs)
    }

    fn push_eval(&self) -> Option<EvalFn> {
        (**self).push_eval()
    }

    fn pull_eval(&self) -> Option<EvalFn> {
        (**self).pull_eval()
    }

    fn stateful(&self) -> bool {
        (**self).stateful()
    }

    fn register(&self, path: &[Id], vm: &mut Engine) {
        (**self).register(path, vm)
    }
}

macro_rules! impl_node_for_ptr {
    ($($Ty:ident)::*) => {
        impl<T> Node for $($Ty)::*<T>
        where
            T: ?Sized + Node,
        {
            fn n_inputs(&self) -> usize {
                (**self).n_inputs()
            }

            fn n_outputs(&self) -> usize {
                (**self).n_outputs()
            }

            fn expr(&self, inputs: &[Option<ExprKind>]) -> ExprKind {
                (**self).expr(inputs)
            }

            fn push_eval(&self) -> Option<EvalFn> {
                (**self).push_eval()
            }

            fn pull_eval(&self) -> Option<EvalFn> {
                (**self).pull_eval()
            }

            fn stateful(&self) -> bool {
                (**self).stateful()
            }

            fn register(&self, path: &[Id], vm: &mut Engine) {
                (**self).register(path, vm)
            }
        }
    };
}

impl_node_for_ptr!(Box);
impl_node_for_ptr!(std::rc::Rc);
impl_node_for_ptr!(std::sync::Arc);

impl From<u16> for Input {
    fn from(u: u16) -> Self {
        Input(u)
    }
}

impl From<u16> for Output {
    fn from(u: u16) -> Self {
        Output(u)
    }
}

/// Create a node from the given Steel expression.
///
/// Shorthand for `node::Expr::new`.
pub fn expr(expr: impl Into<String>) -> Result<Expr, ExprError> {
    Expr::new(expr)
}
