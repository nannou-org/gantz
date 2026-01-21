//! The primary [`Node`] abstraction and related items.

#[doc(inline)]
pub use crate::visit::{self, Visitor};
pub use apply::Apply;
#[doc(inline)]
pub use conns::Conns;
pub use expr::{Expr, ExprError};
pub use fn_::Fn;
use gantz_ca::CaHash;
pub use graph::GraphNode;
pub use identity::{Identity, IDENTITY_NAME};
pub use pull::{Pull, WithPullEval};
pub use push::{Push, WithPushEval};
use serde::{Deserialize, Serialize};
pub use state::{NodeState, State, WithStateType};
use steel::{parser::ast::ExprKind, steel_vm::engine::Engine};

pub mod apply;
mod conns;
pub mod expr;
pub mod fn_;
pub mod graph;
pub mod identity;
pub mod pull;
pub mod push;
pub mod ref_;
pub mod state;

/// The definitive abstraction of a gantz graph, the gantz `Node` trait.
///
/// The `Env` input parameter allows for providing a shared input to all
/// nodes throughout the graph. This can be used for sharing immutable data
/// between nodes without the need for shared references like `Arc` or `Rc`
/// allowing graphs to remain serializable.
pub trait Node<Env> {
    /// The number of inputs to the node.
    ///
    /// The maximum number is [`Conns::MAX`].
    fn n_inputs(&self, _env: &Env) -> usize {
        0
    }

    /// The number of outputs from the node.
    ///
    /// The maximum number is [`Conns::MAX`].
    fn n_outputs(&self, _env: &Env) -> usize {
        0
    }

    /// The list of possible branches from this node.
    ///
    /// Each branch is represented as a set of outputs that are enabled for that
    /// branch.
    ///
    /// This is intended for nodes that conditionally activate outputs based on
    /// some received input.
    ///
    /// If the returned `Vec` is empty, we assume the node has no branching, and
    /// simply evaluates to all outputs.
    ///
    /// If the returned `Vec` is non-empty, the expression returned from
    /// [`Node::expr`] method must return a list with two elements where the
    /// first element is the index of the selected branch, and the second
    /// element is the node's output value(s).
    ///
    /// By default, this is `vec![]`.
    fn branches(&self, _env: &Env) -> Vec<EvalConf> {
        vec![]
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
    fn expr(&self, ctx: ExprCtx<Env>) -> ExprKind;

    /// Specifies whether or not code should be generated to allow for push
    /// evaluation from instances of this node. Enabling push evaluation allows
    /// applications to call into the graph by calling the resulting generated
    /// code at runtime.
    ///
    /// Push evaluation order is equivalent to a topological ordering of the
    /// connected component that starts from the `push_eval` node.
    ///
    /// Within a **Graph** node, a new function will be generated for each
    /// `EvalConf` set for each node. If **Some**, a function will be generated
    /// with the given **Signature** that represents pushing evaluation from
    /// this node.
    ///
    /// By default, this is an empty vec.
    fn push_eval(&self, _env: &Env) -> Vec<EvalConf> {
        vec![]
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
    /// By default, this is an empty vec.
    fn pull_eval(&self, _env: &Env) -> Vec<EvalConf> {
        vec![]
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
    /// within their [`Node::expr`] provided during compilation.
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
    fn visit(&self, _ctx: visit::Ctx<Env>, _visitor: &mut dyn Visitor<Env>) {}
}

/// A set of connections over which to push/pull evaluation.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub enum EvalConf {
    /// Requires a fn for evaluation from all connections.
    #[default]
    All,
    /// Requires a fn for evaluation from a subset of the connections.
    ///
    /// An element for each connection, `true` if eval-enabled.
    Set(Conns),
}

/// Type used to represent a node's ID within a graph.
pub type Id = usize;

/// Context provided to the [`Node::expr`] fn.
pub struct ExprCtx<'a, Env> {
    /// Access to the environment provided to the node.
    env: &'a Env,
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
    /// An element for each input to the node.
    ///
    /// If the input is connected, it is `Some(name)` where `name` is a binding
    /// to the incoming value.
    outputs: &'a Conns,
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

impl<'a, Env> ExprCtx<'a, Env> {
    pub(crate) fn new(
        env: &'a Env,
        path: &'a [Id],
        inputs: &'a [Option<String>],
        outputs: &'a Conns,
    ) -> Self {
        Self {
            env,
            path,
            inputs,
            outputs,
        }
    }

    /// Access the environment provided to the node.
    pub fn env(&self) -> &Env {
        self.env
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

    /// An element for each output from the node.
    ///
    /// If an output is `true`, it means a value is expected for the output.
    ///
    /// Note that even if an output is connected, it may not be `true` if it is
    /// not included in the eval path.
    ///
    /// Note that even if
    ///
    /// If the output is connected, it is `true`.
    pub fn outputs(&self) -> &Conns {
        self.outputs
    }
}

impl<'a, Env, N> Node<Env> for &'a N
where
    N: ?Sized + Node<Env>,
{
    fn n_inputs(&self, env: &Env) -> usize {
        (**self).n_inputs(env)
    }

    fn n_outputs(&self, env: &Env) -> usize {
        (**self).n_outputs(env)
    }

    fn branches(&self, env: &Env) -> Vec<EvalConf> {
        (**self).branches(env)
    }

    fn expr(&self, ctx: ExprCtx<Env>) -> ExprKind {
        (**self).expr(ctx)
    }

    fn push_eval(&self, env: &Env) -> Vec<EvalConf> {
        (**self).push_eval(env)
    }

    fn pull_eval(&self, env: &Env) -> Vec<EvalConf> {
        (**self).pull_eval(env)
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

    fn visit(&self, ctx: visit::Ctx<Env>, visitor: &mut dyn Visitor<Env>) {
        (**self).visit(ctx, visitor)
    }
}

macro_rules! impl_node_for_ptr {
    ($($Ty:ident)::*) => {
        impl<Env, T> Node<Env> for $($Ty)::*<T>
        where
            T: ?Sized + Node<Env>,
        {
            fn n_inputs(&self, env: &Env) -> usize {
                (**self).n_inputs(env)
            }

            fn n_outputs(&self, env: &Env) -> usize {
                (**self).n_outputs(env)
            }

            fn branches(&self, env: &Env) -> Vec<EvalConf> {
                (**self).branches(env)
            }

            fn expr(&self, ctx: ExprCtx<Env>) -> ExprKind {
                (**self).expr(ctx)
            }

            fn push_eval(&self, env: &Env) -> Vec<EvalConf> {
                (**self).push_eval(env)
            }

            fn pull_eval(&self, env: &Env) -> Vec<EvalConf> {
                (**self).pull_eval(env)
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

            fn visit(&self, ctx: visit::Ctx<Env>, visitor: &mut dyn Visitor<Env>) {
                (**self).visit(ctx, visitor)
            }
        }
    };
}

impl_node_for_ptr!(Box);
impl_node_for_ptr!(std::rc::Rc);
impl_node_for_ptr!(std::sync::Arc);

impl<'a, Env> Clone for ExprCtx<'a, Env> {
    fn clone(&self) -> Self {
        Self {
            env: self.env,
            path: self.path,
            inputs: self.inputs,
            outputs: self.outputs,
        }
    }
}

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

impl CaHash for EvalConf {
    fn hash(&self, hasher: &mut gantz_ca::Hasher) {
        const ALL_TAG: u8 = 0;
        const SET_TAG: u8 = 1;
        match self {
            Self::All => {
                hasher.update(&[ALL_TAG]);
            }
            Self::Set(set) => {
                hasher.update(&[SET_TAG]);
                set.hash(hasher);
            }
        }
    }
}

/// Create a node from the given Steel expression.
///
/// Shorthand for `node::Expr::new`.
pub fn expr(expr: impl Into<String>) -> Result<Expr, ExprError> {
    Expr::new(expr)
}

/// Visit this node and all nested nodes.
pub fn visit<Env>(ctx: visit::Ctx<Env>, node: &dyn Node<Env>, visitor: &mut dyn Visitor<Env>) {
    visitor.visit_pre(ctx, node);
    node.visit(ctx, visitor);
    visitor.visit_post(ctx, node);
}

/// Register the given node and all nested nodes.
pub fn register<Env>(ctx: visit::Ctx<Env>, node: &dyn Node<Env>, vm: &mut Engine) {
    visit(ctx, node, &mut visit::Register(vm));
}
