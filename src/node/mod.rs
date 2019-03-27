use super::{Deserialize, Fail, From, Serialize};

pub mod expr;
pub mod push;
pub mod serde;

pub use self::expr::{Expr, NewExprError};
pub use self::push::{Push, WithPushEval};
pub use self::serde::SerdeNode;

/// Gantz allows for constructing executable directed graphs by composing together **Node**s.
/// 
/// **Node**s are a way to allow users to abstract and encapsulate logic into smaller, re-usable
/// components, similar to a function in a coded programming language.
/// 
/// Every Node is made up of the following:
/// 
/// - Any number of inputs, where each input is of some rust type or generic type.
/// - Any number of outputs, where each output is of some rust type or generic type.
/// - A function that takes the inputs as arguments and returns an Outputs struct containing a
///   field for each of the outputs.
pub trait Node {
    /// The number of inputs to the node.
    fn n_inputs(&self) -> u32;

    /// The number of outputs to the node.
    fn n_outputs(&self) -> u32;

    /// Tokens representing the rust code that will evaluate to a tuple containing all outputs.
    ///
    /// TODO: Consider making `args` a `Vec` of `Option`s and returning an `Option` expr to allow
    /// for generating execution paths where only a certain set of inputs have been triggered.
    /// Returning `None` could indicate that there is no valid `Expr` for the current set of
    /// triggered inputs. This would probably be better than than using `default` as is currently
    /// the case. Would also allow for 
    fn expr(&self, args: Vec<syn::Expr>) -> syn::Expr;

    /// Specifies whether or not code should be generated to allow for push evaluation from
    /// instances of this node. Enabling push evaluation allows applications to call into
    /// the gantz graph by loading the resulting generated code at runtime.
    ///
    /// Within a **Graph** node, a new function will be generated for each node that signals
    /// **Some**.  If **Some**, a function will be generated with the given **FnDecl** that
    /// represents pushing evaluation from this node.
    ///
    /// Gantz will **panic!** if the returned **FnDecl** has a return type other than `()`.
    ///
    /// By default, this is **None**.
    fn push_eval(&self) -> Option<PushEval> {
        None
    }

    /// The same as `push_eval` but allows for generating code for pull evaluation instead.
    ///
    /// *TODO: Finish this.*
    fn pull_eval(&self) -> Option<PullEval> {
        None
    }
}

/// Items that need to be known in order to generate a push evaluation function for a node.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct PushEval {
    /// The type for each argument.
    #[serde(with = "crate::node::serde::fn_decl")]
    pub fn_decl: syn::FnDecl,
    /// The name for the function.
    pub fn_name: String,
    /// Attributes for the generated `ItemFn`.
    #[serde(with = "crate::node::serde::fn_attrs")]
    pub fn_attrs: Vec<syn::Attribute>,
}

/// Items that need to be known in order to generate a pull evaluation function for a node.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PullEval;

/// Represents an input of a node via an index.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct Input(pub u32);

/// Represents an output of a node via an index.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct Output(pub u32);

impl<'a, N> Node for &'a N
where
    N: Node,
{
    fn n_inputs(&self) -> u32 {
        (**self).n_inputs()
    }

    fn n_outputs(&self) -> u32 {
        (**self).n_outputs()
    }

    fn expr(&self, args: Vec<syn::Expr>) -> syn::Expr {
        (**self).expr(args)
    }

    fn push_eval(&self) -> Option<PushEval> {
        (**self).push_eval()
    }

    fn pull_eval(&self) -> Option<PullEval> {
        (**self).pull_eval()
    }
}

macro_rules! impl_node_for_ptr {
    ($($Ty:ident)::*) => {
        impl Node for $($Ty)::*<Node> {
            fn n_inputs(&self) -> u32 {
                (**self).n_inputs()
            }

            fn n_outputs(&self) -> u32 {
                (**self).n_outputs()
            }

            fn expr(&self, args: Vec<syn::Expr>) -> syn::Expr {
                (**self).expr(args)
            }

            fn push_eval(&self) -> Option<PushEval> {
                (**self).push_eval()
            }

            fn pull_eval(&self) -> Option<PullEval> {
                (**self).pull_eval()
            }
        }
    };
}

impl_node_for_ptr!(Box);
impl_node_for_ptr!(std::rc::Rc);
impl_node_for_ptr!(std::sync::Arc);

impl From<syn::ItemFn> for PushEval {
    fn from(item_fn: syn::ItemFn) -> Self {
        let syn::ItemFn { attrs: fn_attrs, decl, ident, .. } = item_fn;
        let fn_decl = *decl;
        let fn_name = format!("{}", ident);
        PushEval { fn_decl, fn_name, fn_attrs }
    }
}

impl From<u32> for Input {
    fn from(u: u32) -> Self {
        Input(u)
    }
}

impl From<u32> for Output {
    fn from(u: u32) -> Self {
        Output(u)
    }
}

/// Create a node from the given Rust expression.
///
/// Shorthand for `node::Expr::new`.
pub fn expr(expr: &str) -> Result<Expr, NewExprError> {
    Expr::new(expr)
}
