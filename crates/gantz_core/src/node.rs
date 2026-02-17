//! The primary [`Node`] abstraction and related items.

#[doc(inline)]
pub use crate::visit::{self, Visitor};
pub use apply::Apply;
#[doc(inline)]
pub use conns::Conns;
pub use expr::{Expr, ExprNewError};
pub use fn_::Fn;
use gantz_ca::CaHash;
pub use graph::GraphNode;
pub use id::{IDENTITY_NAME, Identity};
pub use pull::{Pull, WithPullEval};
pub use push::{Push, WithPushEval};
pub use ref_::Ref;
use serde::{Deserialize, Serialize};
pub use state::{NodeState, State, WithStateType};
use steel::{parser::ast::ExprKind, steel_vm::engine::Engine};

pub mod apply;
mod conns;
pub mod expr;
pub mod fn_;
pub mod graph;
pub mod id;
pub mod pull;
pub mod push;
pub mod ref_;
pub mod state;

/// The definitive abstraction of a gantz graph, the gantz `Node` trait.
pub trait Node {
    /// The number of inputs to the node.
    ///
    /// The maximum number is [`Conns::MAX`].
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        0
    }

    /// The number of outputs from the node.
    ///
    /// The maximum number is [`Conns::MAX`].
    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
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
    fn branches(&self, _ctx: MetaCtx) -> Vec<EvalConf> {
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
    fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult;

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
    fn push_eval(&self, _ctx: MetaCtx) -> Vec<EvalConf> {
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
    fn pull_eval(&self, _ctx: MetaCtx) -> Vec<EvalConf> {
        vec![]
    }

    /// Whether or not this node acts as an inlet for some nested graph.
    fn inlet(&self, _ctx: MetaCtx) -> bool {
        false
    }

    /// Whether or not this node acts as an outlet for some nested graph.
    fn outlet(&self, _ctx: MetaCtx) -> bool {
        false
    }

    /// Whether or not the node requires access to state.
    ///
    /// Nodes returning `true` will have a special `state` variable accessible
    /// within their [`Node::expr`] provided during compilation.
    fn stateful(&self, _ctx: MetaCtx) -> bool {
        false
    }

    /// Function for registering necessary types, functions and initialising any
    /// default values as necessary.
    ///
    /// This method is called each time the graph changes and must be idempotent.
    /// Implementations should check whether state already exists before
    /// initializing to avoid resetting existing state. See
    /// [`state::init_value_if_absent`] and [`state::init_if_absent`].
    ///
    /// Nodes returning `true` from their [`Node::stateful`] implementation
    /// must use this to initialise their state.
    ///
    /// By default, the node is assumed to be stateless, and this does nothing.
    fn register(&self, _ctx: RegCtx<'_, '_>) {}

    /// Returns the content addresses of external nodes this node requires.
    ///
    /// Used during pruning to determine which commits/graphs are still in use.
    /// Nodes that reference other graphs (like `Ref`, `NamedRef`) should return
    /// the addresses they depend on.
    ///
    /// By default, returns an empty vec (no external dependencies).
    fn required_addrs(&self) -> Vec<gantz_ca::ContentAddr> {
        vec![]
    }

    /// Traverse all nested nodes, depth-first, with the given [`Visitor`].
    ///
    /// For each nested node:
    ///
    /// 1. `Visitor::visit_pre`
    /// 2. `Node::visit`
    /// 3. `Visitor::visit_post`
    ///
    /// Note that implementations should *only* visit nested nodes and not the
    /// node itself. To visit the node *and* all nested nodes, use the [`visit()`]
    /// function.
    fn visit(&self, _ctx: visit::Ctx<'_, '_>, _visitor: &mut dyn Visitor) {}
}

/// A set of connections over which to push/pull evaluation.
#[derive(Clone, Debug, Default, Deserialize, Serialize, CaHash)]
#[cahash("gantz.eval-conf")]
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

/// Type alias for the node lookup function.
///
/// Used by context types to allow looking up nodes by content address.
pub type GetNode<'a> = &'a dyn std::ops::Fn(&gantz_ca::ContentAddr) -> Option<&'a dyn Node>;

/// Context for node metadata queries (`n_inputs`, `n_outputs`, `stateful`, etc.).
#[derive(Clone, Copy)]
pub struct MetaCtx<'a> {
    get_node: GetNode<'a>,
}

/// Context for node registration (registering state, functions with VM).
pub struct RegCtx<'env, 'data> {
    get_node: GetNode<'env>,
    path: &'data [Id],
    vm: &'data mut Engine,
}

/// Context provided to the [`Node::expr`] fn.
pub struct ExprCtx<'env, 'data> {
    /// Function for looking up nodes by content address.
    get_node: GetNode<'env>,
    /// The path of this node relative to the root of the gantz graph.
    ///
    /// This is primarily provided to allow `GraphNode`s (or custom graph node
    /// implementations) to generate the correct function names for their
    /// nested nodes.
    ///
    /// Besides this special case, `path` should not be used so that node's
    /// maintain consistent behaviour whether nested or not.
    path: &'data [Id],
    /// An element for each input to the node.
    ///
    /// If the input is connected, it is `Some(name)` where `name` is a binding
    /// to the incoming value.
    inputs: &'data [Option<String>],
    /// An element for each output from the node.
    ///
    /// If an output is `true`, it means a value is expected for the output.
    outputs: &'data Conns,
}

/// Represents a function that can be called to begin evaluation of the graph
/// from some node.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EvalFn;

/// Represents an input of a node via an index.
#[derive(
    Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, Deserialize, Serialize, CaHash,
)]
pub struct Input(pub u16);

/// Represents an output of a node via an index.
#[derive(
    Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, Deserialize, Serialize, CaHash,
)]
pub struct Output(pub u16);

/// Error during expression generation.
#[derive(Clone, Debug, thiserror::Error)]
#[error("{0}")]
pub struct ExprError(Box<str>);

/// Result type for expression generation.
pub type ExprResult = Result<ExprKind, ExprError>;

impl<'a> MetaCtx<'a> {
    /// Create a new metadata context with the given node lookup function.
    pub fn new(get_node: GetNode<'a>) -> Self {
        Self { get_node }
    }

    /// Look up a node by content address.
    pub fn node(&self, ca: &gantz_ca::ContentAddr) -> Option<&'a dyn Node> {
        (self.get_node)(ca)
    }
}

impl<'env, 'data> RegCtx<'env, 'data> {
    /// Create a new registration context.
    pub fn new(get_node: GetNode<'env>, path: &'data [Id], vm: &'data mut Engine) -> Self {
        Self { get_node, path, vm }
    }

    /// Look up a node by content address.
    pub fn node(&self, ca: &gantz_ca::ContentAddr) -> Option<&'env dyn Node> {
        (self.get_node)(ca)
    }

    /// The path of this node relative to the root of the gantz graph.
    pub fn path(&self) -> &'data [Id] {
        self.path
    }

    /// Access to the node lookup function.
    pub fn get_node(&self) -> GetNode<'env> {
        self.get_node
    }

    /// Mutable access to the Steel VM.
    pub fn vm(&mut self) -> &mut Engine {
        self.vm
    }

    /// Decompose the context into its parts.
    pub fn into_parts(self) -> (GetNode<'env>, &'data [Id], &'data mut Engine) {
        (self.get_node, self.path, self.vm)
    }
}

impl<'env, 'data> ExprCtx<'env, 'data> {
    pub fn new(
        get_node: GetNode<'env>,
        path: &'data [Id],
        inputs: &'data [Option<String>],
        outputs: &'data Conns,
    ) -> Self {
        Self {
            get_node,
            path,
            inputs,
            outputs,
        }
    }

    /// Look up a node by content address.
    pub fn node(&self, ca: &gantz_ca::ContentAddr) -> Option<&'env dyn Node> {
        (self.get_node)(ca)
    }

    /// The path of this node relative to the root of the gantz graph.
    ///
    /// This is primarily provided to allow `GraphNode`s (or custom graph node
    /// implementations) to generate the correct function names for their
    /// nested nodes.
    ///
    /// Besides this special case, `path` should not be used so that node's
    /// maintain consistent behaviour whether nested or not.
    pub fn path(&self) -> &'data [Id] {
        self.path
    }

    /// An element for each input to the node.
    ///
    /// If the input is connected, it is `Some(name)` where `name` is a binding
    /// to the incoming value.
    pub fn inputs(&self) -> &'data [Option<String>] {
        self.inputs
    }

    /// An element for each output from the node.
    ///
    /// If an output is `true`, it means a value is expected for the output.
    ///
    /// Note that even if an output is connected, it may not be `true` if it is
    /// not included in the eval path.
    pub fn outputs(&self) -> &'data Conns {
        self.outputs
    }

    /// Access to the node lookup function.
    pub fn get_node(&self) -> GetNode<'env> {
        self.get_node
    }
}

impl<N> Node for &N
where
    N: ?Sized + Node,
{
    fn n_inputs(&self, ctx: MetaCtx) -> usize {
        (**self).n_inputs(ctx)
    }

    fn n_outputs(&self, ctx: MetaCtx) -> usize {
        (**self).n_outputs(ctx)
    }

    fn branches(&self, ctx: MetaCtx) -> Vec<EvalConf> {
        (**self).branches(ctx)
    }

    fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult {
        (**self).expr(ctx)
    }

    fn push_eval(&self, ctx: MetaCtx) -> Vec<EvalConf> {
        (**self).push_eval(ctx)
    }

    fn pull_eval(&self, ctx: MetaCtx) -> Vec<EvalConf> {
        (**self).pull_eval(ctx)
    }

    fn inlet(&self, ctx: MetaCtx) -> bool {
        (**self).inlet(ctx)
    }

    fn outlet(&self, ctx: MetaCtx) -> bool {
        (**self).outlet(ctx)
    }

    fn stateful(&self, ctx: MetaCtx) -> bool {
        (**self).stateful(ctx)
    }

    fn register(&self, ctx: RegCtx<'_, '_>) {
        (**self).register(ctx)
    }

    fn required_addrs(&self) -> Vec<gantz_ca::ContentAddr> {
        (**self).required_addrs()
    }

    fn visit(&self, ctx: visit::Ctx<'_, '_>, visitor: &mut dyn Visitor) {
        (**self).visit(ctx, visitor)
    }
}

macro_rules! impl_node_for_ptr {
    ($($Ty:ident)::*) => {
        impl<T> Node for $($Ty)::*<T>
        where
            T: ?Sized + Node,
        {
            fn n_inputs(&self, ctx: MetaCtx) -> usize {
                (**self).n_inputs(ctx)
            }

            fn n_outputs(&self, ctx: MetaCtx) -> usize {
                (**self).n_outputs(ctx)
            }

            fn branches(&self, ctx: MetaCtx) -> Vec<EvalConf> {
                (**self).branches(ctx)
            }

            fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult {
                (**self).expr(ctx)
            }

            fn push_eval(&self, ctx: MetaCtx) -> Vec<EvalConf> {
                (**self).push_eval(ctx)
            }

            fn pull_eval(&self, ctx: MetaCtx) -> Vec<EvalConf> {
                (**self).pull_eval(ctx)
            }

            fn inlet(&self, ctx: MetaCtx) -> bool {
                (**self).inlet(ctx)
            }

            fn outlet(&self, ctx: MetaCtx) -> bool {
                (**self).outlet(ctx)
            }

            fn stateful(&self, ctx: MetaCtx) -> bool {
                (**self).stateful(ctx)
            }

            fn register(&self, ctx: RegCtx<'_, '_>) {
                (**self).register(ctx)
            }

            fn required_addrs(&self) -> Vec<gantz_ca::ContentAddr> {
                (**self).required_addrs()
            }

            fn visit(&self, ctx: visit::Ctx<'_, '_>, visitor: &mut dyn Visitor) {
                (**self).visit(ctx, visitor)
            }
        }
    };
}

impl_node_for_ptr!(Box);
impl_node_for_ptr!(std::rc::Rc);
impl_node_for_ptr!(std::sync::Arc);

impl<'env, 'data> Clone for ExprCtx<'env, 'data> {
    fn clone(&self) -> Self {
        Self {
            get_node: self.get_node,
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

impl ExprError {
    /// Create an error from any displayable value.
    pub fn custom(msg: impl std::fmt::Display) -> Self {
        Self(msg.to_string().into_boxed_str())
    }
}

/// Create a node from the given Steel expression.
///
/// Shorthand for `node::Expr::new`.
pub fn expr(expr: impl Into<String>) -> Result<Expr, ExprNewError> {
    Expr::new(expr)
}

/// Parse a Steel expression string, returning an [`ExprResult`].
pub fn parse_expr(src: &str) -> ExprResult {
    let exprs = Engine::emit_ast(src).map_err(|e| ExprError::custom(e))?;
    exprs
        .into_iter()
        .next()
        .ok_or_else(|| ExprError::custom("empty expression"))
}

/// Visit this node and all nested nodes.
pub fn visit(ctx: visit::Ctx<'_, '_>, node: &dyn Node, visitor: &mut dyn Visitor) {
    visitor.visit_pre(ctx, node);
    node.visit(ctx, visitor);
    visitor.visit_post(ctx, node);
}

/// Register the given node and all nested nodes.
pub fn register(ctx: visit::Ctx<'_, '_>, node: &dyn Node, vm: &mut Engine) {
    visit(ctx, node, &mut visit::Register(vm));
}
