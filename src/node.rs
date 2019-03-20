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
#[typetag::serde(tag = "type")]
pub trait Node {
    /// The number of inputs to the node.
    fn n_inputs(&self) -> u32;

    /// The number of outputs to the node.
    fn n_outputs(&self) -> u32;

    /// Tokens representing the rust code that will evaluate to an instance of `Self::Outputs`.
    fn expr_tokens(&self, args: Vec<syn::Expr>) -> syn::Expr;
}

/// Represents an input of a node via an index.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct Input(pub u32);

/// Represents an output of a node via an index.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct Output(pub u32);

// pub enum Kind {
//     /// A rust expression of some sort.
//     ///
//     /// - **fn** - inputs are function arguments, output is the return type.
//     /// - **const** - single output nodes that always output the same value.
//     RustExpr,
//     /// A graph (or "subgraph") with inlets and outlets.
//     GantzGraph,
// }
