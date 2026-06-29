//! Pluggable keyword sugar for the `.gantz` text format.
//!
//! Sugar is the layer of human-friendly node keywords (`expr`, `inlet`, ...)
//! over the universal generic form `(node "Tag" (field datum)...)`. The format
//! *core* reserves `ref`/`fn-ref` (references), `graph` (inline nesting) and
//! `node` (the generic fallback) - those need format or `gantz_core` context and
//! are never pluggable. Everything else is provided by a [`Sugar`]: [`CoreSugar`]
//! carries `gantz_core`'s own node set, and each downstream crate implements
//! `Sugar` for its nodes and composes them via [`Sugars`].
//!
//! A `Sugar` only ever deals in tag *strings* and serde [`Datum`]s (read through
//! [`SugarArgs`] and built with [`node_datum`]), so it adds no dependency on the
//! concrete node crates and never sees the raw s-expression AST.
//!
//! Which sugars an application uses is a static property of its node-type
//! universe: [`NodeSugar`] ties a top-level node type to its composite, and the
//! convenience entry points read it via `N::sugar()`.

use crate::datum::{Datum, node_datum};
use crate::error::{ErrorKind, FormatError};
use crate::sexpr::{self, as_keyword, as_string, as_symbol, err_at, quote, span_src};
use steel::parser::ast::ExprKind;

/// The arguments of a list-headed sugar form `(<head> <args>...)`, with helpers
/// to read keyword and positional values without touching the raw s-expression
/// AST. Constructed by the format and handed to [`Sugar::read_spec`].
#[derive(Clone, Copy)]
pub struct SugarArgs<'a> {
    args: &'a [ExprKind],
    src: &'a str,
}

impl<'a> SugarArgs<'a> {
    /// Wrap a slice of argument datums and the source they were read from.
    ///
    /// The format builds this for [`Sugar::read_spec`]; it is also exposed so a
    /// `Sugar` can be unit-tested in isolation (read args with [`crate::sexpr`]).
    pub fn new(args: &'a [ExprKind], src: &'a str) -> Self {
        SugarArgs { args, src }
    }

    /// The number of arguments.
    pub fn count(&self) -> usize {
        self.args.len()
    }

    /// Find a `#:<key>` keyword and return the following integer value.
    pub fn keyword_int(&self, key: &str) -> Result<Option<i64>, FormatError> {
        match self.keyword_at(key) {
            Some((i, kw)) => Ok(Some(
                self.args
                    .get(i + 1)
                    .map(|v| parse_int(v, self.src))
                    .transpose()?
                    .ok_or_else(|| {
                        err_at(
                            kw,
                            self.src,
                            ErrorKind::Malformed(format!("#:{key} requires an integer")),
                        )
                    })?,
            )),
            None => Ok(None),
        }
    }

    /// Find a `#:<key>` keyword and return the following float value.
    pub fn keyword_f64(&self, key: &str) -> Result<Option<f64>, FormatError> {
        match self.keyword_at(key) {
            Some((i, kw)) => Ok(Some(
                self.args
                    .get(i + 1)
                    .map(|v| parse_f64(v, self.src))
                    .transpose()?
                    .ok_or_else(|| {
                        err_at(
                            kw,
                            self.src,
                            ErrorKind::Malformed(format!("#:{key} requires a number")),
                        )
                    })?,
            )),
            None => Ok(None),
        }
    }

    /// Whether a bare `#:<key>` flag keyword is present.
    pub fn has_flag(&self, key: &str) -> bool {
        self.args
            .iter()
            .any(|a| as_keyword(a).as_deref() == Some(key))
    }

    /// The `n`-th positional argument as a string literal.
    pub fn str_at(&self, n: usize) -> Option<String> {
        self.args.get(n).and_then(as_string)
    }

    /// The `n`-th positional argument as a symbol (e.g. a log level).
    pub fn symbol_at(&self, n: usize) -> Option<String> {
        self.args.get(n).and_then(as_symbol)
    }

    /// The `n`-th positional argument as an integer.
    pub fn int_at(&self, n: usize) -> Result<Option<i64>, FormatError> {
        self.args.get(n).map(|e| parse_int(e, self.src)).transpose()
    }

    /// The `n`-th positional argument as a float.
    pub fn f64_at(&self, n: usize) -> Result<Option<f64>, FormatError> {
        self.args.get(n).map(|e| parse_f64(e, self.src)).transpose()
    }

    /// The verbatim source slice of the `n`-th argument.
    ///
    /// The load-bearing primitive for code-carrying forms (`expr`/`branch`):
    /// embedded Steel is captured byte-for-byte so node `src` strings - and the
    /// content addresses that hash them - are preserved exactly.
    pub fn verbatim_at(&self, n: usize) -> Option<&'a str> {
        self.args.get(n).and_then(|e| span_src(e, self.src))
    }

    /// A malformed-form error located at the `n`-th argument (unlocated if the
    /// argument is absent).
    pub fn malformed_at(&self, n: usize, msg: impl Into<String>) -> FormatError {
        match self.args.get(n) {
            Some(e) => err_at(e, self.src, ErrorKind::Malformed(msg.into())),
            None => FormatError::malformed(msg),
        }
    }

    /// The index and keyword datum of the first `#:<key>` argument, if present.
    fn keyword_at(&self, key: &str) -> Option<(usize, &'a ExprKind)> {
        self.args
            .iter()
            .enumerate()
            .find(|(_, a)| as_keyword(a).as_deref() == Some(key))
            .map(|(i, a)| (i, a))
    }
}

/// A set of keyword sugars layered over the generic `(node "Tag" ...)` form.
///
/// Reading tries [`Sugar::read_spec`]/[`Sugar::read_bare`]; writing tries
/// [`Sugar::write_spec`], falling back to the generic form. The reserved core
/// heads (`ref`/`fn-ref`/`graph`/`node`) are matched by the reader *before* any
/// sugar, so a sugar cannot shadow them.
pub trait Sugar {
    /// Read a list-headed sugar form `(<head> <args>...)` into a node datum.
    /// `Ok(None)` means this sugar does not recognise `head` (try the next).
    fn read_spec(&self, head: &str, args: SugarArgs<'_>) -> Result<Option<Datum>, FormatError>;

    /// Read a bare keyword (a unit node, e.g. `inlet`) into a node datum.
    fn read_bare(&self, keyword: &str) -> Option<Datum>;

    /// Write a node datum (whose `type` is `tag`) as a sugared form. `None`
    /// falls back to the generic `(node "Tag" ...)` form.
    fn write_spec(&self, tag: &str, node: &Datum) -> Option<String>;

    /// The label stem to generate for a node with this tag (e.g. `inlet`); used
    /// when naming nodes during serialization.
    fn keyword_for_tag(&self, tag: &str) -> Option<&str>;
}

/// Treat a reference to a sugar as a sugar, so `&S`/`&dyn Sugar` compose freely.
impl<S: Sugar + ?Sized> Sugar for &S {
    fn read_spec(&self, head: &str, args: SugarArgs<'_>) -> Result<Option<Datum>, FormatError> {
        (**self).read_spec(head, args)
    }

    fn read_bare(&self, keyword: &str) -> Option<Datum> {
        (**self).read_bare(keyword)
    }

    fn write_spec(&self, tag: &str, node: &Datum) -> Option<String> {
        (**self).write_spec(tag, node)
    }

    fn keyword_for_tag(&self, tag: &str) -> Option<&str> {
        (**self).keyword_for_tag(tag)
    }
}

/// An ordered composition of sugars; for each query the first sugar to handle it
/// wins, so earlier entries take precedence.
pub struct Sugars<'a>(pub Vec<&'a dyn Sugar>);

impl Sugar for Sugars<'_> {
    fn read_spec(&self, head: &str, args: SugarArgs<'_>) -> Result<Option<Datum>, FormatError> {
        for sugar in &self.0 {
            if let Some(datum) = sugar.read_spec(head, args)? {
                return Ok(Some(datum));
            }
        }
        Ok(None)
    }

    fn read_bare(&self, keyword: &str) -> Option<Datum> {
        self.0.iter().find_map(|s| s.read_bare(keyword))
    }

    fn write_spec(&self, tag: &str, node: &Datum) -> Option<String> {
        self.0.iter().find_map(|s| s.write_spec(tag, node))
    }

    fn keyword_for_tag(&self, tag: &str) -> Option<&str> {
        self.0.iter().find_map(|s| s.keyword_for_tag(tag))
    }
}

/// The composite keyword sugar for a node-type universe.
///
/// Implemented once on an application's top-level `Box<dyn Node>` to select which
/// [`Sugar`]s its node set uses; the convenience entry points
/// ([`from_str`](crate::from_str)/[`to_string`](crate::to_string)) read it via
/// `N::sugar()`. Use the `_with` variants to pass a sugar explicitly instead.
pub trait NodeSugar {
    /// The composed sugar for this node set.
    fn sugar() -> Sugars<'static>;
}

/// The keyword sugars for `gantz_core`'s built-in node set: `inlet`, `outlet`,
/// `apply`, `delay`, `id`, `expr` and `branch`. Downstream crates provide a
/// [`Sugar`] for their own nodes and compose via [`Sugars`].
#[derive(Clone, Copy, Debug, Default)]
pub struct CoreSugar;

/// Sugar keyword -> typetag tag, for the `gantz_core` builtins that lower to a
/// plain serde object with no extra arguments. Order is the canonical display
/// order.
const KEYWORD_TAG: &[(&str, &str)] = &[
    ("inlet", "Inlet"),
    ("outlet", "Outlet"),
    ("apply", "Apply"),
    ("delay", "Delay"),
    ("id", "Identity"),
    ("expr", "Expr"),
    ("branch", "Branch"),
];

/// The typetag tag for a sugar keyword.
fn tag_for_keyword(kw: &str) -> Option<&'static str> {
    KEYWORD_TAG
        .iter()
        .find(|(k, _)| *k == kw)
        .map(|&(_, tag)| tag)
}

/// The sugar keyword for a typetag tag, if one exists.
fn keyword_for_tag(tag: &str) -> Option<&'static str> {
    KEYWORD_TAG
        .iter()
        .find(|(_, t)| *t == tag)
        .map(|&(kw, _)| kw)
}

impl Sugar for CoreSugar {
    fn read_spec(&self, head: &str, args: SugarArgs<'_>) -> Result<Option<Datum>, FormatError> {
        let datum = match head {
            "inlet" => inlet_outlet_spec("Inlet", args),
            "outlet" => inlet_outlet_spec("Outlet", args),
            "expr" => expr_spec(args)?,
            "branch" => branch_spec(args)?,
            _ => return Ok(None),
        };
        Ok(Some(datum))
    }

    fn read_bare(&self, keyword: &str) -> Option<Datum> {
        tag_for_keyword(keyword).map(|tag| node_datum(tag, vec![]))
    }

    fn write_spec(&self, tag: &str, node: &Datum) -> Option<String> {
        match tag {
            "Inlet" => Some(write_inlet_outlet("inlet", node)),
            "Outlet" => Some(write_inlet_outlet("outlet", node)),
            "Expr" => Some(write_expr(node)),
            "Branch" => Some(write_branch(node)),
            other => keyword_for_tag(other).map(str::to_string),
        }
    }

    fn keyword_for_tag(&self, tag: &str) -> Option<&str> {
        keyword_for_tag(tag)
    }
}

// -- built-in reading --------------------------------------------------------

/// Read an `(inlet [ty [description]])` / `(outlet ...)` form: two optional
/// string args carrying the socket's hover-doc type label and description.
/// Empty strings are omitted so they serialize as the struct's defaults.
fn inlet_outlet_spec(tag: &str, args: SugarArgs<'_>) -> Datum {
    let mut fields = Vec::new();
    if let Some(ty) = args.str_at(0).filter(|s| !s.is_empty()) {
        fields.push(("ty", Datum::Str(ty)));
    }
    if let Some(desc) = args.str_at(1).filter(|s| !s.is_empty()) {
        fields.push(("description", Datum::Str(desc)));
    }
    node_datum(tag, fields)
}

fn expr_spec(args: SugarArgs<'_>) -> Result<Datum, FormatError> {
    let code = args.verbatim_at(0).ok_or_else(|| match args.count() {
        0 => FormatError::malformed("expr requires code"),
        _ => args.malformed_at(0, "could not slice expr code"),
    })?;
    let mut fields = vec![("src", Datum::Str(code.to_string()))];
    if let Some(out) = args.keyword_int("out")? {
        fields.push(("outputs", Datum::U64(out.max(0) as u64)));
    }
    Ok(node_datum("Expr", fields))
}

fn branch_spec(args: SugarArgs<'_>) -> Result<Datum, FormatError> {
    let code = args.verbatim_at(0).ok_or_else(|| match args.count() {
        0 => FormatError::malformed("branch requires code"),
        _ => args.malformed_at(0, "could not slice branch code"),
    })?;
    let mut masks = Vec::with_capacity(args.count().saturating_sub(1));
    for n in 1..args.count() {
        let mask = args
            .str_at(n)
            .ok_or_else(|| args.malformed_at(n, "branch mask must be a string"))?;
        masks.push(Datum::Str(mask));
    }
    Ok(node_datum(
        "Branch",
        vec![
            ("src", Datum::Str(code.to_string())),
            ("branches", Datum::Seq(masks)),
        ],
    ))
}

// -- built-in writing --------------------------------------------------------

fn write_expr(node: &Datum) -> String {
    let src = node.get("src").and_then(Datum::as_str).unwrap_or("'()");
    match node.get("outputs").and_then(Datum::as_i64) {
        Some(n) if n != 1 => format!("(expr {src} #:out {n})"),
        _ => format!("(expr {src})"),
    }
}

fn write_branch(node: &Datum) -> String {
    let src = node.get("src").and_then(Datum::as_str).unwrap_or("'()");
    let masks = node
        .get("branches")
        .and_then(Datum::as_seq)
        .map(|a| {
            a.iter()
                .filter_map(Datum::as_str)
                .map(quote)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    format!("(branch {src} {masks})")
}

/// Write an `Inlet`/`Outlet` as a bare keyword when it carries no socket docs,
/// else as `(inlet ty [description])` so the hover docs round-trip.
fn write_inlet_outlet(keyword: &str, node: &Datum) -> String {
    let ty = node.get("ty").and_then(Datum::as_str).unwrap_or("");
    let desc = node
        .get("description")
        .and_then(Datum::as_str)
        .unwrap_or("");
    match (ty.is_empty(), desc.is_empty()) {
        (true, true) => keyword.to_string(),
        (false, true) => format!("({keyword} {})", quote(ty)),
        (_, false) => format!("({keyword} {} {})", quote(ty), quote(desc)),
    }
}

// -- helpers -----------------------------------------------------------------

fn parse_int(e: &ExprKind, src: &str) -> Result<i64, FormatError> {
    sexpr::as_i64(e, src)
        .ok_or_else(|| err_at(e, src, ErrorKind::Malformed("expected an integer".into())))
}

fn parse_f64(e: &ExprKind, src: &str) -> Result<f64, FormatError> {
    sexpr::as_f64(e, src)
        .ok_or_else(|| err_at(e, src, ErrorKind::Malformed("expected a number".into())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A custom sugar for a hypothetical node set: `(gain <db>)` and bare `mute`.
    struct GainSugar;

    impl Sugar for GainSugar {
        fn read_spec(&self, head: &str, args: SugarArgs<'_>) -> Result<Option<Datum>, FormatError> {
            if head != "gain" {
                return Ok(None);
            }
            let db = args.int_at(0)?.unwrap_or(0);
            Ok(Some(node_datum("Gain", vec![("db", Datum::I64(db))])))
        }

        fn read_bare(&self, keyword: &str) -> Option<Datum> {
            (keyword == "mute").then(|| node_datum("Mute", vec![]))
        }

        fn write_spec(&self, tag: &str, node: &Datum) -> Option<String> {
            match tag {
                "Gain" => {
                    let db = node.get("db").and_then(Datum::as_i64).unwrap_or(0);
                    Some(format!("(gain {db})"))
                }
                "Mute" => Some("mute".to_string()),
                _ => None,
            }
        }

        fn keyword_for_tag(&self, tag: &str) -> Option<&str> {
            match tag {
                "Gain" => Some("gain"),
                "Mute" => Some("mute"),
                _ => None,
            }
        }
    }

    fn read_spec(sugar: &dyn Sugar, text: &str) -> Option<Datum> {
        let exprs = sexpr::read(text).expect("read");
        let args = sexpr::list_args(&exprs[0]).expect("list");
        let head = sexpr::as_symbol(&args[0]).expect("head");
        sugar
            .read_spec(&head, SugarArgs::new(&args[1..], text))
            .expect("read_spec")
    }

    #[test]
    fn custom_sugar_reads_and_writes() {
        let s = GainSugar;
        let datum = read_spec(&s, "(gain 6)").expect("recognised");
        assert_eq!(datum.get("type").and_then(Datum::as_str), Some("Gain"));
        assert_eq!(datum.get("db").and_then(Datum::as_i64), Some(6));
        assert_eq!(s.write_spec("Gain", &datum).as_deref(), Some("(gain 6)"));
        assert_eq!(
            s.write_spec("Mute", &node_datum("Mute", vec![])).as_deref(),
            Some("mute")
        );
        assert_eq!(s.read_bare("mute"), Some(node_datum("Mute", vec![])));
    }

    #[test]
    fn sugars_compose_first_hit_wins() {
        let (gain, core) = (GainSugar, CoreSugar);
        let sugars = Sugars(vec![&gain, &core]);

        // The custom keyword is handled by GainSugar.
        let g = read_spec(&sugars, "(gain 3)").expect("gain recognised");
        assert_eq!(g.get("db").and_then(Datum::as_i64), Some(3));

        // A built-in keyword still resolves through the composed CoreSugar.
        let e = read_spec(&sugars, "(expr (+ 1 2))").expect("expr recognised");
        assert_eq!(e.get("type").and_then(Datum::as_str), Some("Expr"));

        // keyword_for_tag composes across both sugars.
        assert_eq!(sugars.keyword_for_tag("Gain"), Some("gain"));
        assert_eq!(sugars.keyword_for_tag("Inlet"), Some("inlet"));
    }

    #[test]
    fn custom_only_sugar_falls_through_to_generic() {
        // A custom-only sugar that does not know the built-ins: reads return
        // None (so the reader would reject the keyword) and writes return None
        // (so the writer emits the generic `(node ...)` form).
        let s = GainSugar;
        assert!(
            s.read_spec("expr", SugarArgs::new(&[], ""))
                .expect("ok")
                .is_none()
        );
        assert!(s.read_bare("inlet").is_none());
        assert!(
            s.write_spec("Inlet", &node_datum("Inlet", vec![]))
                .is_none()
        );
        assert!(s.keyword_for_tag("Inlet").is_none());
    }

    #[test]
    fn inlet_outlet_docs_round_trip() {
        let s = CoreSugar;

        // Both type and description.
        let d = read_spec(&s, r#"(inlet "number" "left operand")"#).expect("recognised");
        assert_eq!(d.get("type").and_then(Datum::as_str), Some("Inlet"));
        assert_eq!(d.get("ty").and_then(Datum::as_str), Some("number"));
        assert_eq!(
            d.get("description").and_then(Datum::as_str),
            Some("left operand"),
        );
        assert_eq!(
            s.write_spec("Inlet", &d).as_deref(),
            Some(r#"(inlet "number" "left operand")"#),
        );

        // Type only.
        let ty_only = read_spec(&s, r#"(inlet "number")"#).expect("ty only");
        assert_eq!(
            s.write_spec("Inlet", &ty_only).as_deref(),
            Some(r#"(inlet "number")"#),
        );

        // Outlet round-trips the same way.
        let o = read_spec(&s, r#"(outlet "list" "the reversed list")"#).expect("outlet");
        assert_eq!(
            s.write_spec("Outlet", &o).as_deref(),
            Some(r#"(outlet "list" "the reversed list")"#),
        );

        // A bare (undocumented) inlet still writes as the bare keyword.
        let bare = s.read_bare("inlet").expect("bare inlet");
        assert_eq!(s.write_spec("Inlet", &bare).as_deref(), Some("inlet"));
    }
}
