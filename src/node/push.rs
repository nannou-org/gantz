use crate::node::{self, Node};

/// A wrapper around a `Node` that enables push evaluation.
///
/// The implementation of `Node` will match the inner node type `N`, but with a unique
/// implementation of `Node::push_eval`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Push<N> {
    node: N,
    push_eval: node::PushEval,
}

/// A trait implemented for all `Node` types allowing to enable push evaluation.
pub trait WithPushEval: Sized + Node {
    /// Consume `self` and return a `Node` that has push evaluation enabled.
    fn with_push_eval(self, push_eval: node::PushEval) -> Push<Self>;

    /// Enable push evaluation using the given push evaluation function.
    ///
    /// Internally, this calls `with_push_eval`.
    ///
    /// Note: Only the name, function declaration and attributes are used - the function definition
    /// is ignored.
    fn with_push_eval_fn(self, item_fn: syn::ItemFn) -> Push<Self> {
        self.with_push_eval(item_fn.into())
    }

    /// Enable push evaluation.
    ///
    /// Internally, this calls `with_push_eval_fn` with a function that looks like `fn #name() {}`.
    fn with_push_eval_name(self, fn_name: &str) -> Push<Self> {
        let fn_ident = syn::Ident::new(fn_name, proc_macro2::Span::call_site());
        self.with_push_eval_fn(syn::parse_quote!{ fn #fn_ident() {} })
    }
}

impl<N> Push<N>
where
    N: Node,
{
    /// Given some node, return a `Push` node enabling push evaluation.
    pub fn new(node: N, push_eval: node::PushEval) -> Self {
        Push { node, push_eval }
    }
}

impl<N> WithPushEval for N
where
    N: Node,
{
    /// Consume `self` and return an equivalent node with push evaluation enabled.
    fn with_push_eval(self, push_eval: node::PushEval) -> Push<Self> {
        Push::new(self, push_eval)
    }
}

impl<N> Node for Push<N>
where
    N: Node,
{
    fn n_inputs(&self) -> u32 {
        self.node.n_inputs()
    }

    fn n_outputs(&self) -> u32 {
        self.node.n_outputs()
    }

    fn expr(&self, args: Vec<syn::Expr>) -> syn::Expr {
        self.node.expr(args)
    }

    fn push_eval(&self) -> Option<node::PushEval> {
        Some(self.push_eval.clone())
    }

    fn pull_eval(&self) -> Option<node::PullEval> {
        self.node.pull_eval()
    }
}
