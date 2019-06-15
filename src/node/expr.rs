use super::{Deserialize, Fail, From, Serialize};
use crate::node::{self, Node};
use proc_macro2::{TokenStream, TokenTree};
use quote::{ToTokens, TokenStreamExt};
use std::fmt;
use std::str::FromStr;

/// A simple node that allows for representing rust expressions as nodes within a gantz graph.
///
/// E.g. the following expression:
///
/// ```ignore
/// #freq.sin() * #amp
/// ```
///
/// will result in a single node with two inputs (`#freq` and `#amp`) and a single output which is
/// the result of the expression.
///
/// ## Limitations
///
/// Currently expressions cannot contain any of the following:
///
/// - Attributes, e.g. `{ #[cfg(target_os = "macos")] { 2 + 2 } }` is a valid expr but not allowed.
/// - Raw strings, e.g. `{ r#"blah blah"# }` is a valid expr but not allowed.
/// - Comments containing the `#` token.
///
/// These limitations are caused by the primitive way in which string interpolation is achieved (we
/// simply count each of the occurrences of `#`). This may be improved in the future.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Expr {
    #[serde(with = "crate::node::serde::tts")]
    tokens: TokenStream,
}

/// An error occurred while constructing the `Expr` node.
#[derive(Debug, Fail, From)]
pub enum NewExprError {
    #[fail(display = "failed to parse the `str` as a valid `TokenStream`")]
    InvalidTokenStream,
    #[fail(display = "failed to parse the `str` as a valid expr: {}", err)]
    InvalidExpr {
        #[fail(cause)]
        err: syn::Error,
    },
}

impl Expr {
    /// Construct an **Expr** node from the given rust expression.
    ///
    /// Returns an **Err** if the given string is not a valid expression when interpolated with
    /// valid sub-expressions.
    ///
    /// ```rust
    /// fn main() {
    ///     let _node = gantz::node::Expr::new("#foo + #bar").unwrap();
    /// }
    /// ```
    pub fn new(expr: &str) -> Result<Self, NewExprError> {
        // Retrieve the `TokenStream`.
        let tokens = TokenStream::from_str(expr).map_err(|_| NewExprError::InvalidTokenStream)?;
        // Count the number of inputs.
        let n_inputs = count_hashes(&tokens);
        // Interpolate the `TokenStream` with some temp `{}` expressions.
        let unit_expr: syn::Expr = syn::parse_quote! { {} };
        let test_expr_tokens = interpolate_tokens(&tokens, vec![unit_expr; n_inputs as usize]);
        let test_expr_str = format!("{}", test_expr_tokens);
        let _: syn::Expr = syn::parse_str(&test_expr_str)?;
        // If we got this far, we have a valid `Expr`!
        Ok(Expr { tokens })
    }
}

impl Node for Expr {
    fn evaluator(&self) -> node::Evaluator {
        let n_inputs = count_hashes(&self.tokens);
        let n_outputs = 1;
        let tokens = self.tokens.clone();
        let gen_expr = Box::new(move |args: Vec<syn::Expr>| {
            let args_tokens = args.into_iter().map(|expr| expr.into_token_stream());
            let expr_tokens = interpolate_tokens(&tokens, args_tokens);
            syn::parse_quote! { #expr_tokens }
        });
        node::Evaluator::Expr {
            n_inputs,
            n_outputs,
            gen_expr,
        }
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.tokens)
    }
}

// A `Punct` instance representing a `#`.
fn hash_punct() -> proc_macro2::Punct {
    proc_macro2::Punct::new('#', proc_macro2::Spacing::Alone)
}

// Given a token stream, count all occurrences of `#`.
fn count_hashes<T>(tokens: T) -> u32
where
    T: ToTokens,
{
    let mut count = 0;
    for t in tokens.into_token_stream() {
        match t {
            TokenTree::Punct(ref p) if format!("{}", p) == format!("{}", hash_punct()) => {
                count += 1;
            }
            TokenTree::Group(ref g) => {
                count += count_hashes(g.stream());
            }
            _ => (),
        }
    }
    count
}

// Given a token stream, sequentially replace each occurrence of `#var` with each expression.
fn interpolate_tokens<T, E>(tokens: T, exprs: E) -> TokenStream
where
    T: ToTokens,
    E: IntoIterator,
    E::Item: ToTokens,
{
    fn interpolate_tokens_inner<E>(tokens: TokenStream, exprs: &mut E) -> TokenStream
    where
        E: Iterator,
        E::Item: ToTokens,
    {
        let mut tokens = tokens.into_iter();
        let mut new_tokens = TokenStream::default();
        while let Some(t) = tokens.next() {
            match t {
                TokenTree::Punct(ref p) if format!("{}", p) == format!("{}", hash_punct()) => {
                    if let Some(expr) = exprs.next() {
                        tokens.next();
                        new_tokens.append_all(expr.into_token_stream());
                    }
                }
                TokenTree::Group(g) => {
                    let new_group_tokens = interpolate_tokens_inner(g.stream(), exprs);
                    let new_group = proc_macro2::Group::new(g.delimiter(), new_group_tokens);
                    new_tokens.append(new_group);
                }
                t => new_tokens.append(t),
            }
        }
        new_tokens
    }

    let tokens = tokens.into_token_stream();
    let mut exprs = exprs.into_iter();
    interpolate_tokens_inner(tokens, &mut exprs)
}

#[test]
fn test_count_hashes() {
    let expr = TokenStream::from_str("#l + #r").unwrap();
    assert_eq!(count_hashes(expr), 2);

    let expr = TokenStream::from_str("#freq.sin() * #amp").unwrap();
    assert_eq!(count_hashes(expr), 2);

    let expr = TokenStream::from_str("&#foo").unwrap();
    assert_eq!(count_hashes(expr), 1);

    let expr = TokenStream::from_str("[#a, #b, #c, #d, #e]").unwrap();
    assert_eq!(count_hashes(expr), 5);

    let expr = TokenStream::from_str("{}").unwrap();
    assert_eq!(count_hashes(expr), 0);
}
