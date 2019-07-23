use super::{Deserialize, Serialize};
use crate::node::{self, Node};

/// A wrapper around a `Node` that enables pull evaluation.
///
/// The implementation of `Node` will match the inner node type `N`, but with a unique
/// implementation of `Node::pull_eval`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Pull<N> {
    node: N,
    pull_eval: node::EvalFn,
}

/// A trait implemented for all `Node` types allowing to enable pull evaluation.
pub trait WithPullEval: Sized + Node {
    /// Consume `self` and return a `Node` that has pull evaluation enabled.
    fn with_pull_eval(self, pull_eval: node::EvalFn) -> Pull<Self>;

    /// Enable pull evaluation using the given pull evaluation function.
    ///
    /// Internally, this calls `with_pull_eval`.
    ///
    /// Note: Only the name, function declaration and attributes are used - the function definition
    /// is ignored.
    fn with_pull_eval_fn(self, item_fn: syn::ItemFn) -> Pull<Self> {
        self.with_pull_eval(item_fn.into())
    }

    /// Enable pull evaluation.
    ///
    /// Internally, this calls `with_pull_eval_fn` with a function that looks like `fn #name() {}`.
    fn with_pull_eval_name(self, fn_name: &str) -> Pull<Self> {
        let fn_ident = syn::Ident::new(fn_name, proc_macro2::Span::call_site());
        self.with_pull_eval_fn(syn::parse_quote! { fn #fn_ident() {} })
    }
}

impl<N> Pull<N>
where
    N: Node,
{
    /// Given some node, return a `Pull` node enabling pull evaluation.
    pub fn new(node: N, pull_eval: node::EvalFn) -> Self {
        Pull { node, pull_eval }
    }
}

impl<N> WithPullEval for N
where
    N: Node,
{
    /// Consume `self` and return an equivalent node with pull evaluation enabled.
    fn with_pull_eval(self, pull_eval: node::EvalFn) -> Pull<Self> {
        Pull::new(self, pull_eval)
    }
}

impl<N> Node for Pull<N>
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
        Some(self.pull_eval.clone())
    }

    fn state_type(&self) -> Option<syn::Type> {
        self.node.state_type()
    }

    fn crate_deps(&self) -> Vec<node::CrateDep> {
        self.node.crate_deps()
    }
}
