#[doc(inline)]
pub use crate::visit::{self, Visitor};
pub use expr::{Expr, ExprError};
pub use graph::GraphNode;
pub use pull::{Pull, WithPullEval};
pub use push::{Push, WithPushEval};
use serde::{Deserialize, Serialize};
pub use state::{NodeState, State, WithStateType};
use steel::{parser::ast::ExprKind, steel_vm::engine::Engine};

pub mod expr;
pub mod graph;
pub mod pull;
pub mod push;
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
    fn expr(&self, ctx: ExprCtx) -> ExprKind;

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

    /// Whether or not this node acts as an inlet for some nested graph.
    fn inlet(&self) -> bool {
        false
    }

    /// Whether or not this node acts as an outlet for some nested graph.
    fn outlet(&self) -> bool {
        false
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

    /// Traverse all nested nodes, depth-first, with the given [`Visitor`].
    ///
    /// For each nested node:
    ///
    /// 1. `Visitor::visit_pre`
    /// 2. `Node::visit`
    /// 3. `Visitor::visit_post`
    ///
    /// Note that implementations should *only* visit nested nodes and not the
    /// node itself. To visit the node *and* all nested nodes, use the [`visit`]
    /// function.
    fn visit(&self, _ctx: visit::Ctx, _visitor: &mut dyn Visitor) {}
}

/// Type used to represent a node's ID within a graph.
pub type Id = usize;

/// Context provided to the [`Node::expr`] fn.
pub struct ExprCtx<'a> {
    /// The path of this node relative to the root of the gantz graph.
    ///
    /// This is primarily provided to allow `GraphNode`s (or custom graph node
    /// implementations) to generate the correct function names for their
    /// nested nodes.
    ///
    /// Besides this special case, `path` should not be used so that node's
    /// maintain consistent behaviour whether nested or not.
    path: &'a [Id],
    /// An element for each input to the node.
    ///
    /// If the input is connected, it is `Some(name)` where `name` is a binding
    /// to the incoming value.
    inputs: &'a [Option<String>],
}

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

impl<'a> ExprCtx<'a> {
    pub(crate) fn new(path: &'a [Id], inputs: &'a [Option<String>]) -> Self {
        Self { path, inputs }
    }

    /// The path of this node relative to the root of the gantz graph.
    ///
    /// This is primarily provided to allow `GraphNode`s (or custom graph node
    /// implementations) to generate the correct function names for their
    /// nested nodes.
    ///
    /// Besides this special case, `path` should not be used so that node's
    /// maintain consistent behaviour whether nested or not.
    pub fn path(&self) -> &[Id] {
        self.path
    }

    /// An element for each input to the node.
    ///
    /// If the input is connected, it is `Some(name)` where `name` is a binding
    /// to the incoming value.
    pub fn inputs(&self) -> &[Option<String>] {
        self.inputs
    }
}

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

    fn expr(&self, ctx: ExprCtx) -> ExprKind {
        (**self).expr(ctx)
    }

    fn push_eval(&self) -> Option<EvalFn> {
        (**self).push_eval()
    }

    fn pull_eval(&self) -> Option<EvalFn> {
        (**self).pull_eval()
    }

    fn inlet(&self) -> bool {
        (**self).inlet()
    }

    fn outlet(&self) -> bool {
        (**self).outlet()
    }

    fn stateful(&self) -> bool {
        (**self).stateful()
    }

    fn register(&self, path: &[Id], vm: &mut Engine) {
        (**self).register(path, vm)
    }

    fn visit(&self, ctx: visit::Ctx, visitor: &mut dyn Visitor) {
        (**self).visit(ctx, visitor)
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

            fn expr(&self, ctx: ExprCtx) -> ExprKind {
                (**self).expr(ctx)
            }

            fn push_eval(&self) -> Option<EvalFn> {
                (**self).push_eval()
            }

            fn pull_eval(&self) -> Option<EvalFn> {
                (**self).pull_eval()
            }

            fn inlet(&self) -> bool {
                (**self).inlet()
            }

            fn outlet(&self) -> bool {
                (**self).outlet()
            }

            fn stateful(&self) -> bool {
                (**self).stateful()
            }

            fn register(&self, path: &[Id], vm: &mut Engine) {
                (**self).register(path, vm)
            }

            fn visit(&self, ctx: visit::Ctx, visitor: &mut dyn Visitor) {
                (**self).visit(ctx, visitor)
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

/// Visit this node and all nested nodes.
pub fn visit(ctx: visit::Ctx, node: &dyn Node, visitor: &mut dyn Visitor) {
    visitor.visit_pre(ctx, node);
    node.visit(ctx, visitor);
    visitor.visit_post(ctx, node);
}

/// Register the given node and all nested nodes.
pub fn register(ctx: visit::Ctx, node: &dyn Node, vm: &mut Engine) {
    visit(ctx, node, &mut visit::Register(vm));
}
