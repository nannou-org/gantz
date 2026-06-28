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
///
/// ## Trigger input
///
/// An expression always has at least one input. When it contains no `$vars`
/// (e.g. `(list 1 2 3)`), it still exposes a single *trigger* input whose value
/// is ignored; a push into it simply forces the expression to evaluate. This
/// makes it easy to author constant values that fire on demand without a dummy
/// `$bang` variable.
///
/// ## Optional inputs
///
/// Variables prefixed with `$?` are treated as optional inputs. When
/// connected, the value is wrapped as `(Some value)`. When unconnected,
/// `(None)` is substituted. This uses Steel's built-in Option type, so
/// `Some?`, `None?`, and `Some->value` are available.
///
/// ```ignore
/// (+ $a (if (Some? $?b) (Some->value $?b) 0))
/// ```
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, CaHash)]
#[cahash("gantz.expr")]
pub struct Expr {
    src: String,
    outputs: u8,
    /// Unique `$` variable names in order of first appearance (cached).
    /// Skipped during serialization and recomputed on deserialization.
    #[serde(skip)]
    #[cahash(skip)]
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
            #[serde(default = "default_outputs")]
            outputs: u8,
        }
        let data = ExprData::deserialize(deserializer)?;
        if data.outputs < 1 || data.outputs > 16 {
            return Err(serde::de::Error::custom(format!(
                "outputs must be in 1..=16, got {}",
                data.outputs,
            )));
        }
        let vars = vars_from_src(&data.src);
        Ok(Expr {
            src: data.src,
            outputs: data.outputs,
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
        Ok(Expr {
            src,
            outputs: 1,
            vars,
        })
    }

    /// Set the number of outputs for this expression node.
    ///
    /// When `n > 1`, the expression should evaluate to `(list v1 v2 ...)`.
    ///
    /// # Panics
    ///
    /// Panics if `n` is 0 or greater than 16.
    pub fn with_outputs(mut self, n: u8) -> Self {
        assert!(n >= 1 && n <= 16, "outputs must be in 1..=16, got {n}");
        self.outputs = n;
        self
    }

    /// The number of outputs for this expression node.
    pub fn outputs(&self) -> u8 {
        self.outputs
    }

    /// Set the number of outputs in place.
    ///
    /// # Panics
    ///
    /// Panics if `n` is 0 or greater than 16.
    pub fn set_outputs(&mut self, n: u8) {
        assert!(n >= 1 && n <= 16, "outputs must be in 1..=16, got {n}");
        self.outputs = n;
    }

    /// The source string that was used to create this node.
    pub fn src(&self) -> &str {
        &self.src
    }

    /// The unique `$` variable names, in order of first appearance.
    ///
    /// Each name maps to the input at the same index, including the `$` or
    /// `$?` (optional) prefix.
    pub fn vars(&self) -> &[String] {
        &self.vars
    }
}

fn default_outputs() -> u8 {
    1
}

/// Collect unique `$var` names in order of first appearance.
pub(crate) fn collect_unique_vars(tts: TokenStream) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut vars = Vec::new();
    for token in tts {
        // steel 0.8's `TokenStream` yields `Result`s; read the `source` field
        // (present on both the clean and the lex-error token) so we still see
        // every token's text.
        let src = match &token {
            Ok(t) => t.source,
            Err(e) => e.source,
        };
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
pub(crate) fn vars_from_src(src: &str) -> Vec<String> {
    collect_unique_vars(TokenStream::new(src, true, None))
}

/// Replace each `$var` with the corresponding input expression based on variable name.
///
/// Variables with a `$?` prefix are treated as optional inputs:
/// - Connected: substituted with `(Some <binding>)`.
/// - Unconnected: substituted with `(None)`.
///
/// Regular `$var` variables use the existing behaviour:
/// - Connected: substituted with the binding name.
/// - Unconnected: substituted with `'()`.
pub(crate) fn interpolate_tokens(
    tts: TokenStream,
    vars: &[String],
    inputs: &[Option<String>],
) -> String {
    // Build a map from var name to input index.
    let var_to_index: std::collections::HashMap<&str, usize> = vars
        .iter()
        .enumerate()
        .map(|(i, v)| (v.as_str(), i))
        .collect();

    let tokens = tts.map(|token| {
        // See `collect_unique_vars`: 0.8's `TokenStream` yields `Result`s, so
        // reconstruct from the `source` field of every token.
        let src = match &token {
            Ok(t) => t.source,
            Err(e) => e.source,
        };
        if src.starts_with("$?") {
            let index = var_to_index.get(src).unwrap();
            match inputs.get(*index).and_then(|o| o.as_ref()) {
                None => "(None)".to_string(),
                Some(in_expr) => format!("(Some {in_expr})"),
            }
        } else if src.starts_with("$") {
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

impl Node for Expr {
    fn n_inputs(&self, _ctx: node::MetaCtx) -> usize {
        // Always expose at least one input. With no `$vars`, the single input
        // is a trigger whose value is ignored (see the type docs).
        self.vars.len().max(1)
    }

    fn n_outputs(&self, _ctx: node::MetaCtx) -> usize {
        self.outputs as usize
    }

    fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
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
    fn stateful(&self, _ctx: node::MetaCtx) -> bool {
        self.src().contains("state")
    }

    /// Registers a state slot just in case `state` is referenced by the expr.
    fn register(&self, ctx: node::RegCtx<'_, '_>) {
        let (_, path, vm) = ctx.into_parts();
        node::state::init_value_if_absent(vm, path, || steel::SteelVal::Void).unwrap();
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

    // Optional vars are captured with $? prefix.
    let vars = vars_from_src("(+ $?a $b)");
    assert_eq!(vars, vec!["$?a", "$b"]);

    // Duplicate optional vars deduplicate.
    let vars = vars_from_src("(if $?x $?x 0)");
    assert_eq!(vars, vec!["$?x"]);

    // Mixed required and optional.
    let vars = vars_from_src("(+ $a $?b $?c)");
    assert_eq!(vars, vec!["$a", "$?b", "$?c"]);
}

// A parse or lex error must surface as an `Err`, never be silently accepted.
// `emit_ast` is the parse gate in both `Expr::new` and `Expr::expr`, so the
// token reconstruction in `collect_unique_vars`/`interpolate_tokens` - which
// just copies each token's source, including steel 0.8's fallible `Err` tokens
// - cannot swallow an error: `emit_ast` re-lexes the result and propagates it.
#[test]
fn invalid_expr_source_is_rejected() {
    // Construction gate (`Expr::new` -> `emit_ast(&src)?`).
    assert!(Expr::new("(+ $a").is_err(), "unbalanced paren");
    assert!(
        Expr::new("\"unterminated").is_err(),
        "lex error: unterminated string"
    );
    assert!(Expr::new("").is_err(), "no expression");

    // Compile gate (`Expr::expr` -> `emit_ast(&new_src)?`). A malformed source
    // can reach `expr` via deserialization, which doesn't re-validate, so
    // compilation itself must error rather than emit broken Steel.
    let bad: Expr = serde_json::from_str(r#"{"src":"(+ 1","outputs":1}"#).unwrap();
    let outputs = node::Conns::try_from([true]).unwrap();
    let ctx = node::ExprCtx::new(&|_| None, &[], &[None], &outputs);
    assert!(bad.expr(ctx).is_err(), "malformed src must fail to compile");
}

// A no-op node lookup for constructing a `MetaCtx` in tests.
#[cfg(test)]
fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
    None
}

#[test]
fn test_n_inputs_min_one() {
    let ctx = || node::MetaCtx::new(&no_lookup);
    // No `$vars` - still exposes a single trigger input.
    assert_eq!(Expr::new("(list 1 2 3)").unwrap().n_inputs(ctx()), 1);
    assert_eq!(Expr::new("42").unwrap().n_inputs(ctx()), 1);
    // A single var.
    assert_eq!(Expr::new("$foo").unwrap().n_inputs(ctx()), 1);
    // Multiple vars are unaffected.
    assert_eq!(Expr::new("(+ $l $r)").unwrap().n_inputs(ctx()), 2);
}

#[test]
fn test_outputs_default() {
    let e = Expr::new("(+ $a $b)").unwrap();
    assert_eq!(e.outputs(), 1);
}

#[test]
fn test_with_outputs() {
    let e = Expr::new("(values $a $b)").unwrap().with_outputs(2);
    assert_eq!(e.outputs(), 2);
}

#[test]
fn test_set_outputs() {
    let mut e = Expr::new("(values $a $b $c)").unwrap();
    e.set_outputs(3);
    assert_eq!(e.outputs(), 3);
}

#[test]
#[should_panic]
fn test_outputs_zero_panics() {
    Expr::new("$a").unwrap().with_outputs(0);
}

#[test]
#[should_panic]
fn test_outputs_exceeds_max_panics() {
    Expr::new("$a").unwrap().with_outputs(17);
}

#[test]
fn test_interpolate_optional_unconnected() {
    let src = "(if $?a $?a 0)";
    let vars = vars_from_src(src);
    let tts = TokenStream::new(src, true, None);
    let result = interpolate_tokens(tts, &vars, &[None]);
    assert!(result.contains("(None)"), "expected (None) in: {result}");
}

#[test]
fn test_interpolate_optional_connected() {
    let src = "(if $?a $?a 0)";
    let vars = vars_from_src(src);
    let tts = TokenStream::new(src, true, None);
    let result = interpolate_tokens(tts, &vars, &[Some("input0".into())]);
    assert!(
        result.contains("(Some input0)"),
        "expected (Some input0) in: {result}",
    );
}

#[test]
fn test_interpolate_mixed_required_optional() {
    let src = "(+ $a (unwrap-or $?b 0))";
    let vars = vars_from_src(src);
    let tts = TokenStream::new(src, true, None);
    // $a connected, $?b unconnected.
    let result = interpolate_tokens(tts, &vars, &[Some("input0".into()), None]);
    assert!(result.contains("input0"), "expected input0 in: {result}");
    assert!(result.contains("(None)"), "expected (None) in: {result}");
}

#[test]
fn test_interpolate_required_unconnected_unchanged() {
    let src = "(+ $a $b)";
    let vars = vars_from_src(src);
    let tts = TokenStream::new(src, true, None);
    let result = interpolate_tokens(tts, &vars, &[Some("input0".into()), None]);
    assert!(result.contains("input0"), "expected input0 in: {result}");
    assert!(result.contains("'()"), "expected '() in: {result}");
    // Ensure no Option wrapping for required vars.
    assert!(
        !result.contains("(None)"),
        "should not contain (None): {result}"
    );
    assert!(
        !result.contains("(Some"),
        "should not contain (Some: {result}"
    );
}
