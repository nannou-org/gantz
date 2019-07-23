use super::{Deserialize, Serialize};

pub mod expr;
pub mod pull;
pub mod push;
pub mod serde;
pub mod state;

pub use self::expr::{Expr, NewExprError};
pub use self::pull::{Pull, WithPullEval};
pub use self::push::{Push, WithPushEval};
pub use self::serde::SerdeNode;
pub use self::state::{State, WithStateType};

/// Gantz allows for constructing executable directed graphs by composing together **Node**s.
///
/// **Node**s are a way to allow users to abstract and encapsulate logic into smaller, re-usable
/// components, similar to a function in a coded programming language.
///
/// Every Node is made up of the following:
///
/// - Any number of inputs, where each input is of some rust type or generic type.
/// - Any number of outputs, where each output is of some rust type or generic type.
/// - An expression that takes the inputs as arguments and returns the outputs (via a tuple in
///   the case of more than one).
pub trait Node {
    /// The approach taken for evaluating a nodes inputs to its outputs.
    ///
    /// This can either be an expression or a function - the key difference being that the types of
    /// a function's inputs and outputs are known before compilation begins. As a result, functions
    /// can lead to gantz generating more modular, compiler-friendly code, while raw expressions
    /// have the benefit of being more ergonomic for the implementer as types aren't resolved until
    /// the compilation process begins.
    fn evaluator(&self) -> Evaluator;

    /// Specifies whether or not code should be generated to allow for push evaluation from
    /// instances of this node. Enabling push evaluation allows applications to call into
    /// the gantz graph by loading the resulting generated code at runtime.
    ///
    /// Push evaluation order is equivalent to a topological ordering of the connected component
    /// that starts from the `push_eval` node.
    ///
    /// Within a **Graph** node, a new function will be generated for each node that signals
    /// **Some**.  If **Some**, a function will be generated with the given **FnDecl** that
    /// represents pushing evaluation from this node.
    ///
    /// Gantz will **panic!** if the returned **FnDecl** has a return type other than `()`.
    ///
    /// By default, this is **None**.
    fn push_eval(&self) -> Option<EvalFn> {
        None
    }

    /// Specifies whether or not code should be generated to allow for pull evaluation from
    /// instances of this node. Enabling pull evaluation allows applications to call into
    /// the gantz graph by loading the resulting generated code at runtime.
    ///
    /// Pull evaluation order is equivalent to a topological ordering of the connected component
    /// that ends at the `pull_eval` node.
    ///
    /// Within a **Graph** node, a new function will be generated for each node that signals
    /// **Some**.  If **Some**, a function will be generated with the given **FnDecl** that
    /// represents pulling evaluation from this node.
    ///
    /// Gantz will **panic!** if the returned **FnDecl** has a return type other than `()`.
    ///
    /// By default, this is **None**.
    fn pull_eval(&self) -> Option<EvalFn> {
        None
    }

    /// If the node type requires access to some persistent state when evaluating its expression,
    /// return the expected type of that state here.
    ///
    /// Code generation will ensure that a local binding named `state` of type `&mut T` (where `T`
    /// is the type returned by this function) will be available to the node's expression.
    ///
    /// By default, this is **None** indicating a stateless node.
    fn state_type(&self) -> Option<syn::Type> {
        None
    }
}

/// The method of evaluation used for a node.
///
/// The key distinction between the `Fn` and `Expr` variants is whether or not types of the inputs
/// and outputs are known before a node is connected to a graph or if instead these types should be
/// inferred.
pub enum Evaluator {
    /// Functions have the benefit of knowing the types of their inputs and outputs.
    ///
    /// Knowing the types of a node's inputs and outputs allow us to:
    ///
    /// - Generate more modular code for a node.
    /// - Create better user feedback and error messages.
    /// - Implement `Node` for `Graph`.
    Fn {
        /// A free-standing function, including its name, declaration, the block and other
        /// attributes.
        fn_item: syn::ItemFn,
    },
    /// Expressions have the benefit of not needing to know the exact types of a node's inputs and
    /// outputs. This simplifies the implementation of the `Node` trait for users.
    Expr {
        /// The function for producing an expression given the input expressions.
        gen_expr: Box<dyn Fn(Vec<syn::Expr>) -> syn::Expr>,
        /// The number of inputs to the expression.
        n_inputs: u32,
        /// The number of outputs to the expression.
        n_outputs: u32,
    },
}

/// Items that need to be known in order to generate a push evaluation function for a node.
///
/// Note that all function declarations will have a single `node_states: node::States` argument
/// appended to their `inputs` list in order to ensure the state associated with each node may be
/// passed down the call stack. This means that when loading the symbol for the
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct EvalFn {
    /// The type for each argument.
    #[serde(with = "crate::node::serde::fn_decl")]
    pub fn_decl: syn::FnDecl,
    /// The name for the function.
    pub fn_name: String,
    /// Attributes for the generated `ItemFn`.
    #[serde(with = "crate::node::serde::fn_attrs")]
    pub fn_attrs: Vec<syn::Attribute>,
}

/// Represents an input of a node via an index.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct Input(pub u32);

/// Represents an output of a node via an index.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct Output(pub u32);

impl Evaluator {
    /// The number of inputs to the node.
    pub fn n_inputs(&self) -> u32 {
        match *self {
            Evaluator::Fn { ref fn_item } => count_fn_inputs(&fn_item.decl) as _,
            Evaluator::Expr { n_inputs, .. } => n_inputs as _,
        }
    }

    /// The number of outputs to the node.
    pub fn n_outputs(&self) -> u32 {
        match *self {
            Evaluator::Fn { ref fn_item } => count_fn_outputs(&fn_item.decl) as _,
            Evaluator::Expr { n_outputs, .. } => n_outputs as _,
        }
    }

    /// Tokens representing the rust code that will evaluate to a tuple containing all outputs.
    ///
    /// TODO: Handle case where only a subset of inputs are connected. See issue #17.
    pub fn expr(&self, args: Vec<syn::Expr>) -> syn::Expr {
        match *self {
            Evaluator::Fn { ref fn_item } => fn_call_expr(fn_item, args),
            Evaluator::Expr { ref gen_expr, .. } => (*gen_expr)(args),
        }
    }
}

impl<'a, N> Node for &'a N
where
    N: Node,
{
    fn evaluator(&self) -> Evaluator {
        (**self).evaluator()
    }

    fn push_eval(&self) -> Option<EvalFn> {
        (**self).push_eval()
    }

    fn pull_eval(&self) -> Option<EvalFn> {
        (**self).pull_eval()
    }
}

macro_rules! impl_node_for_ptr {
    ($($Ty:ident)::*) => {
        impl Node for $($Ty)::*<dyn Node> {
            fn evaluator(&self) -> Evaluator {
                (**self).evaluator()
            }

            fn push_eval(&self) -> Option<EvalFn> {
                (**self).push_eval()
            }

            fn pull_eval(&self) -> Option<EvalFn> {
                (**self).pull_eval()
            }
        }
    };
}

impl_node_for_ptr!(Box);
impl_node_for_ptr!(std::rc::Rc);
impl_node_for_ptr!(std::sync::Arc);

impl From<syn::ItemFn> for EvalFn {
    fn from(item_fn: syn::ItemFn) -> Self {
        let syn::ItemFn {
            attrs: fn_attrs,
            decl,
            ident,
            ..
        } = item_fn;
        let fn_decl = *decl;
        let fn_name = format!("{}", ident);
        EvalFn {
            fn_decl,
            fn_name,
            fn_attrs,
        }
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

// Count the number of arguments to the given function.
//
// This is used to determine the number of inputs to the function.
fn count_fn_inputs(fn_decl: &syn::FnDecl) -> usize {
    fn_decl.inputs.len()
}

// Count the number of arguments to the given function.
//
// This is used to determine the number of inputs to the function.
fn count_fn_outputs(fn_decl: &syn::FnDecl) -> usize {
    match fn_decl.output {
        syn::ReturnType::Default => 0,
        syn::ReturnType::Type(ref _r_arrow, ref ty) => match **ty {
            syn::Type::Tuple(ref tuple) => tuple.elems.len(),
            _ => 1,
        },
    }
}

// Create a rust expression that calls the given `fn_decl` function with the given `args`
// expressions as its inputs.
fn fn_call_expr(fn_item: &syn::ItemFn, args: Vec<syn::Expr>) -> syn::Expr {
    let n_inputs = count_fn_inputs(&fn_item.decl);
    assert_eq!(
        n_inputs,
        args.len(),
        "the number of args to a function node must match n_inputs"
    );
    let ident = fn_item.ident.clone();
    let arguments = syn::PathArguments::None;
    let segment = syn::PathSegment { ident, arguments };
    let segments = std::iter::once(segment).collect();
    let leading_colon = None;
    let path = syn::Path {
        leading_colon,
        segments,
    };
    let attrs = vec![];
    let qself = None;
    let func_path = syn::ExprPath { attrs, qself, path };
    let attrs = vec![];
    let func = Box::new(syn::Expr::Path(func_path));
    let paren_token = syn::token::Paren {
        span: proc_macro2::Span::call_site(),
    };
    let args = args.into_iter().collect();
    let expr_call = syn::ExprCall {
        attrs,
        func,
        paren_token,
        args,
    };
    let expr = syn::Expr::Call(expr_call);
    expr
}
