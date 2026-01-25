use crate::node::{self, Node};
use gantz_ca::CaHash;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fmt, str::FromStr};
use steel::{parser::lexer::TokenStream, steel_vm::engine::Engine};
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
///
/// Variables are identified by unique names - if the same `$var` appears
/// multiple times in the expression, it refers to the same inlet.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
pub struct Expr {
    src: String,
    /// Unique `$` variable names in order of first appearance (cached).
    /// Skipped during serialization and recomputed on deserialization.
    #[serde(skip)]
    vars: Vec<String>,
}

impl<'de> Deserialize<'de> for Expr {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct ExprData {
            src: String,
        }
        let data = ExprData::deserialize(deserializer)?;
        let vars = vars_from_src(&data.src);
        Ok(Expr {
            src: data.src,
            vars,
        })
    }
}

/// An error occurred while constructing the `Expr` node.
#[derive(Debug, Error)]
pub enum ExprNewError {
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
    pub fn new(src: impl Into<String>) -> Result<Self, ExprNewError> {
        let src: String = src.into();
        let vars = vars_from_src(&src);
        // Validate that the source parses successfully.
        let exprs = Engine::emit_ast(&src)?;
        if exprs.is_empty() {
            return Err(ExprNewError::Empty);
        }
        Ok(Expr { src, vars })
    }

    /// The source string that was used to create this node.
    pub fn src(&self) -> &str {
        &self.src
    }
}

/// Collect unique `$var` names in order of first appearance.
fn collect_unique_vars(tts: TokenStream) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut vars = Vec::new();
    for token in tts {
        let src = token.source();
        if src.starts_with("$") {
            let var_name = src.to_string();
            if seen.insert(var_name.clone()) {
                vars.push(var_name);
            }
        }
    }
    vars
}

/// Extract unique vars from a source string.
fn vars_from_src(src: &str) -> Vec<String> {
    collect_unique_vars(TokenStream::new(src, true, None))
}

/// Replace each `$var` with the corresponding input expression based on variable name.
fn interpolate_tokens(tts: TokenStream, vars: &[String], inputs: &[Option<String>]) -> String {
    // Build a map from var name to input index.
    let var_to_index: std::collections::HashMap<&str, usize> = vars
        .iter()
        .enumerate()
        .map(|(i, v)| (v.as_str(), i))
        .collect();

    let tokens = tts.map(|token| {
        let src = token.source();
        if src.starts_with("$") {
            let index = var_to_index.get(src).unwrap();
            match inputs.get(*index).and_then(|o| o.as_ref()) {
                None => "'()".to_string(),
                Some(in_expr) => in_expr.clone(),
            }
        } else {
            src.to_string()
        }
    });
    tokens.collect::<Vec<_>>().join(" ")
}

impl<Env> Node<Env> for Expr {
    fn n_inputs(&self, _: &Env) -> usize {
        self.vars.len()
    }

    fn n_outputs(&self, _: &Env) -> usize {
        1
    }

    fn expr(&self, ctx: node::ExprCtx<Env>) -> node::ExprResult {
        // Create a token stream.
        let tts = TokenStream::new(&self.src, true, None);

        // Replace the `$var`s with their input expressions.
        let new_src = interpolate_tokens(tts, &self.vars, ctx.inputs());

        // Convert the interpolated string to an expr.
        let exprs = steel::steel_vm::engine::Engine::emit_ast(&new_src)
            .map_err(|e| node::ExprError::custom(e))?;

        // If there's one expression, return it.
        if exprs.len() == 1 {
            Ok(exprs.into_iter().next().unwrap())
        // If there are multiple expressions, combine them with begin.
        } else {
            let exprs = exprs
                .iter()
                .map(|expr| format!("{expr}"))
                .collect::<Vec<_>>()
                .join(" ");
            let out_src = format!("(begin {})", exprs);
            node::parse_expr(&out_src)
        }
    }

    /// Only generate the state binding if the expr references `state`.
    fn stateful(&self, _env: &Env) -> bool {
        self.src().contains("state")
    }

    /// Registers a state slot just in case `state` is referenced by the expr.
    fn register(&self, _env: &Env, path: &[super::Id], vm: &mut Engine) {
        node::state::init_value_if_absent(vm, path, || steel::SteelVal::Void).unwrap();
    }
}

impl CaHash for Expr {
    fn hash(&self, hasher: &mut gantz_ca::Hasher) {
        hasher.update("gantz_core::node::Expr".as_bytes());
        hasher.update(self.src.as_bytes());
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.src)
    }
}

impl FromStr for Expr {
    type Err = ExprNewError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

#[test]
fn test_collect_unique_vars() {
    // All unique vars.
    let vars = vars_from_src("(+ $l $r)");
    assert_eq!(vars, vec!["$l", "$r"]);

    // Duplicate vars - should only appear once.
    let vars = vars_from_src("(+ $x $x)");
    assert_eq!(vars, vec!["$x"]);

    // Mixed with duplicates - order of first appearance.
    let vars = vars_from_src("(+ $x $y $x $z $y)");
    assert_eq!(vars, vec!["$x", "$y", "$z"]);

    // No vars.
    let vars = vars_from_src("(+ 1 2)");
    assert_eq!(vars, Vec::<String>::new());

    // Single var.
    let vars = vars_from_src("$foo");
    assert_eq!(vars, vec!["$foo"]);

    // Multiple unique vars.
    let vars = vars_from_src("($a $b $c $d $e)");
    assert_eq!(vars, vec!["$a", "$b", "$c", "$d", "$e"]);
}
