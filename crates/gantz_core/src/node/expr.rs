use super::{Deserialize, Serialize};
use crate::node::{self, Node};
use std::{fmt, str::FromStr};
use steel::{
    parser::{ast::ExprKind, lexer::TokenStream},
    steel_vm::engine::Engine,
};
use thiserror::Error;

/// A simple node that allows for representing expressions as nodes.
///
/// E.g. the following expression:
///
/// ```ignore
/// (+ $foo $bar)
/// ```
///
/// will result in a single node with two inputs (`$foo` and `$bar`) and a
/// single output which is the result of the expression.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct Expr {
    src: String,
    /// The total inputs, derived from the `$` count in the src.
    n_inputs: usize,
    /// The total outputs, i.e. the number of `ExprKind`s in the emitted AST.
    /// FIXME: This isn't consistent with `Node::expr`.
    n_outputs: usize,
}

/// An error occurred while constructing the `Expr` node.
#[derive(Debug, Error)]
pub enum ExprError {
    /// Failed to parse a valid expression.
    #[error("failed to parse a valid expr: {err}")]
    InvalidExpr {
        #[from]
        err: steel::rerrs::SteelErr,
    },
    /// The parsed result contained no expression.
    #[error("parsed result contains no expression")]
    Empty,
}

impl Expr {
    /// Construct an **Expr** node from the given rust expression.
    ///
    /// Returns an **Err** if the given string is not a valid expression when
    /// interpolated with valid sub-expressions.
    ///
    /// ```rust
    /// fn main() {
    ///     let _node = gantz_core::node::Expr::new("(+ $foo $bar)").unwrap();
    /// }
    /// ```
    pub fn new(src: impl Into<String>) -> Result<Self, ExprError> {
        let src: String = src.into();
        // Create a token stream.
        let skip_comments = true;
        let source_id = None;
        let tts = TokenStream::new(&src, skip_comments, source_id);
        let n_inputs = count_dollars(tts);
        // NOTE: We can actually parse here as `$foo` is a valid identifier.
        let exprs = Engine::emit_ast(&src)?;
        let n_outputs = exprs.len();
        Ok(Expr {
            src,
            n_inputs,
            n_outputs,
        })
    }

    /// The source string that was used to create this node.
    pub fn src(&self) -> &str {
        &self.src
    }
}

fn count_dollars(tts: TokenStream) -> usize {
    tts.filter(|token| token.source().starts_with("$")).count()
}

/// Consecutively replace each identifier starting with `$` with the expression
/// in the list in order. Return the resulting tokens.
fn interpolate_tokens(tts: TokenStream, inputs: &[Option<String>]) -> String {
    let mut inputs = inputs.iter();
    let tokens = tts.map(move |token| {
        let mut tts = vec![];
        let in_src;
        if token.source().starts_with("$") {
            let input = inputs.next().unwrap();
            match input.as_ref() {
                None => {
                    tts.extend(TokenStream::new("'()", true, None));
                }
                Some(in_expr) => {
                    in_src = format!("{in_expr}");
                    let in_tts = TokenStream::new(&in_src, true, None);
                    tts.extend(in_tts);
                }
            }
        } else {
            tts.push(token);
        }
        tts.iter()
            .map(|t| format!("{}", t.source()))
            .collect::<Vec<_>>()
            .join(" ")
    });
    tokens.collect::<Vec<_>>().join(" ")
}

impl<Env> Node<Env> for Expr {
    fn n_inputs(&self, _: &Env) -> usize {
        self.n_inputs
    }

    fn n_outputs(&self, _: &Env) -> usize {
        self.n_outputs
    }

    fn expr(&self, ctx: node::ExprCtx<Env>) -> ExprKind {
        // Create a token stream.
        let skip_comments = true;
        let source_id = None;
        let tts = TokenStream::new(&self.src, skip_comments, source_id);

        // Replace the `$var`s with their input expressions.
        let new_src = interpolate_tokens(tts, ctx.inputs());

        // Convert the interpolated string to an expr.
        let exprs = Engine::emit_ast(&new_src).expect("failed to emit AST");

        // If there's one expression, return it.
        if exprs.len() == 1 {
            exprs.into_iter().next().unwrap()
        // If there are multiple expressions, combine them with begin?
        } else {
            let exprs = exprs
                .iter()
                .map(|expr| format!("{expr}"))
                .collect::<Vec<_>>()
                .join(" ");
            let out_src = format!("(begin {})", exprs);
            Engine::emit_ast(&out_src)
                .expect("failed to emit AST")
                .into_iter()
                .next()
                .unwrap()
        }
    }

    /// Only generate the state binding if the expr references `state`.
    fn stateful(&self) -> bool {
        self.src().contains("state")
    }

    /// Registers a state slot just in case `state` is referenced by the expr.
    fn register(&self, path: &[super::Id], vm: &mut Engine) {
        node::state::update_value(vm, path, steel::SteelVal::Void).unwrap();
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.src)
    }
}

impl FromStr for Expr {
    type Err = ExprError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

#[test]
fn test_count_dollars() {
    let expr = TokenStream::new("(+ $l $r)", true, None);
    assert_eq!(count_dollars(expr), 2);

    let expr = TokenStream::new("(* (sin $freq) $amp)", true, None);
    assert_eq!(count_dollars(expr), 2);

    let expr = TokenStream::new("$foo", true, None);
    assert_eq!(count_dollars(expr), 1);

    let expr = TokenStream::new("($a, $b, $c, $d, $e)", true, None);
    assert_eq!(count_dollars(expr), 5);

    let expr = TokenStream::new("()", true, None);
    assert_eq!(count_dollars(expr), 0);
}
