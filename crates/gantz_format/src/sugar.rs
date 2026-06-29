//! Pluggable keyword sugar for the `.gantz` text format.
//!
//! Sugar is the layer of human-friendly node keywords (`expr`, `inlet`, ...)
//! over the universal generic form `(node "Tag" (field datum)...)`. The format
//! *core* reserves `ref`/`fn-ref` (references), `graph` (inline nesting) and
//! `node` (the generic fallback) - those need format or `gantz_core` context and
//! are never pluggable. Everything else is provided by a [`Sugar`]:
//! [`DefaultSugar`] carries gantz's built-in node set, and other node sets can
//! implement `Sugar` and compose via [`Sugars`].
//!
//! A `Sugar` only ever deals in tag *strings* and serde [`Datum`]s, so it adds
//! no dependency on the concrete node crates.

use crate::datum::Datum;
use crate::error::{ErrorKind, FormatError};
use crate::sexpr::{self, as_keyword, as_string, as_symbol, err_at, quote, span_src};
use steel::parser::ast::ExprKind;

/// A set of keyword sugars layered over the generic `(node "Tag" ...)` form.
///
/// Reading tries [`Sugar::read_spec`]/[`Sugar::read_bare`]; writing tries
/// [`Sugar::write_spec`], falling back to the generic form. The reserved core
/// heads (`ref`/`fn-ref`/`graph`/`node`) are matched by the reader *before* any
/// sugar, so a sugar cannot shadow them.
pub trait Sugar {
    /// Read a list-headed sugar form `(<head> <args>...)` into a node datum.
    /// `Ok(None)` means this sugar does not recognise `head` (try the next).
    fn read_spec(
        &self,
        head: &str,
        args: &[ExprKind],
        src: &str,
    ) -> Result<Option<Datum>, FormatError>;

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
    fn read_spec(
        &self,
        head: &str,
        args: &[ExprKind],
        src: &str,
    ) -> Result<Option<Datum>, FormatError> {
        (**self).read_spec(head, args, src)
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
    fn read_spec(
        &self,
        head: &str,
        args: &[ExprKind],
        src: &str,
    ) -> Result<Option<Datum>, FormatError> {
        for sugar in &self.0 {
            if let Some(datum) = sugar.read_spec(head, args, src)? {
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

/// The keyword sugars for gantz's built-in node set.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultSugar;

/// Sugar keyword -> typetag tag, for the built-ins that lower to a plain serde
/// object with no extra arguments. Order is the canonical display order.
const KEYWORD_TAG: &[(&str, &str)] = &[
    ("inlet", "Inlet"),
    ("outlet", "Outlet"),
    ("apply", "Apply"),
    ("delay", "Delay"),
    ("id", "Identity"),
    ("bang", "Bang"),
    ("inspect", "Inspect"),
    ("update-bang", "UpdateBang"),
    ("tick-bang", "TickBang"),
    ("number", "Number"),
    ("log", "Log"),
    ("expr", "Expr"),
    ("branch", "Branch"),
    ("comment", "Comment"),
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

impl Sugar for DefaultSugar {
    fn read_spec(
        &self,
        head: &str,
        args: &[ExprKind],
        src: &str,
    ) -> Result<Option<Datum>, FormatError> {
        let datum = match head {
            "inlet" => inlet_outlet_spec("Inlet", args),
            "outlet" => inlet_outlet_spec("Outlet", args),
            "expr" => expr_spec(args, src)?,
            "branch" => branch_spec(args, src)?,
            "comment" => comment_spec(args, src)?,
            "number" => number_spec(args, src)?,
            "log" => log_spec(args, src)?,
            "tick-bang" => tick_bang_spec(args, src)?,
            _ => return Ok(None),
        };
        Ok(Some(datum))
    }

    fn read_bare(&self, keyword: &str) -> Option<Datum> {
        match keyword {
            "log" => Some(node_datum(
                "Log",
                vec![("level", Datum::Str("INFO".into()))],
            )),
            _ => tag_for_keyword(keyword).map(|tag| node_datum(tag, vec![])),
        }
    }

    fn write_spec(&self, tag: &str, node: &Datum) -> Option<String> {
        match tag {
            "Inlet" => Some(write_inlet_outlet("inlet", node)),
            "Outlet" => Some(write_inlet_outlet("outlet", node)),
            "Expr" => Some(write_expr(node)),
            "Branch" => Some(write_branch(node)),
            "Comment" => Some(write_comment(node)),
            "Number" => Some(write_number(node)),
            "Log" => Some(write_log(node)),
            "TickBang" => Some(write_tick_bang(node)),
            other => keyword_for_tag(other).map(str::to_string),
        }
    }

    fn keyword_for_tag(&self, tag: &str) -> Option<&str> {
        keyword_for_tag(tag)
    }
}

// -- built-in reading --------------------------------------------------------

/// Build a node datum from a typetag tag and ordered fields - an `&str`-keyed
/// convenience over [`Datum::tagged`].
fn node_datum(tag: &str, fields: Vec<(&str, Datum)>) -> Datum {
    Datum::tagged(
        tag,
        fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

/// Read an `(inlet [ty [description]])` / `(outlet ...)` form: two optional
/// string args carrying the socket's hover-doc type label and description.
/// Empty strings are omitted so they serialize as the struct's defaults.
fn inlet_outlet_spec(tag: &str, args: &[ExprKind]) -> Datum {
    let mut fields = Vec::new();
    if let Some(ty) = args.first().and_then(as_string).filter(|s| !s.is_empty()) {
        fields.push(("ty", Datum::Str(ty)));
    }
    if let Some(desc) = args.get(1).and_then(as_string).filter(|s| !s.is_empty()) {
        fields.push(("description", Datum::Str(desc)));
    }
    node_datum(tag, fields)
}

fn expr_spec(args: &[ExprKind], src: &str) -> Result<Datum, FormatError> {
    let code = args
        .first()
        .ok_or_else(|| FormatError::new(ErrorKind::Malformed("expr requires code".into())))?;
    let code_src = span_src(code, src).ok_or_else(|| {
        err_at(
            code,
            src,
            ErrorKind::Malformed("could not slice expr code".into()),
        )
    })?;
    let mut fields = vec![("src", Datum::Str(code_src.to_string()))];
    if let Some(out) = keyword_int(&args[1..], "out", src)? {
        fields.push(("outputs", Datum::U64(out.max(0) as u64)));
    }
    Ok(node_datum("Expr", fields))
}

fn branch_spec(args: &[ExprKind], src: &str) -> Result<Datum, FormatError> {
    let code = args
        .first()
        .ok_or_else(|| FormatError::new(ErrorKind::Malformed("branch requires code".into())))?;
    let code_src = span_src(code, src).ok_or_else(|| {
        err_at(
            code,
            src,
            ErrorKind::Malformed("could not slice branch code".into()),
        )
    })?;
    let masks: Vec<Datum> = args[1..]
        .iter()
        .map(|m| {
            as_string(m).map(Datum::Str).ok_or_else(|| {
                err_at(
                    m,
                    src,
                    ErrorKind::Malformed("branch mask must be a string".into()),
                )
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(node_datum(
        "Branch",
        vec![
            ("src", Datum::Str(code_src.to_string())),
            ("branches", Datum::Seq(masks)),
        ],
    ))
}

fn comment_spec(args: &[ExprKind], src: &str) -> Result<Datum, FormatError> {
    let text = args
        .first()
        .and_then(as_string)
        .ok_or_else(|| FormatError::new(ErrorKind::Malformed("comment requires text".into())))?;
    let [w, h] = match (args.get(1), args.get(2)) {
        (Some(w), Some(h)) => [int_at(w, src)?.max(0) as u64, int_at(h, src)?.max(0) as u64],
        _ => [100, 40],
    };
    Ok(node_datum(
        "Comment",
        vec![
            ("text", Datum::Str(text)),
            ("size", Datum::Seq(vec![Datum::U64(w), Datum::U64(h)])),
        ],
    ))
}

/// Read a `(number [#:min m] [#:max m] [#:precision n] [#:no-push-eval])` form.
/// Only the non-default fields are emitted, so a bare `number` stays bare.
fn number_spec(args: &[ExprKind], src: &str) -> Result<Datum, FormatError> {
    let mut fields = Vec::new();
    if let Some(min) = keyword_f64(args, "min", src)? {
        fields.push(("min", Datum::F64(min)));
    }
    if let Some(max) = keyword_f64(args, "max", src)? {
        fields.push(("max", Datum::F64(max)));
    }
    if let Some(precision) = keyword_int(args, "precision", src)? {
        fields.push(("precision", Datum::U64(precision.max(0) as u64)));
    }
    if has_flag(args, "no-push-eval") {
        fields.push(("push_eval_on_edit", Datum::Bool(false)));
    }
    Ok(node_datum("Number", fields))
}

fn log_spec(args: &[ExprKind], src: &str) -> Result<Datum, FormatError> {
    let level = match args.first().and_then(as_symbol) {
        Some(s) => log_level(&s, &args[0], src)?,
        None => "INFO".to_string(),
    };
    Ok(node_datum("Log", vec![("level", Datum::Str(level))]))
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

fn write_comment(node: &Datum) -> String {
    let text = node.get("text").and_then(Datum::as_str).unwrap_or("");
    let (w, h) = node
        .get("size")
        .and_then(Datum::as_seq)
        .and_then(|a| Some((a.first()?.as_i64()?, a.get(1)?.as_i64()?)))
        .unwrap_or((100, 40));
    format!("(comment {} {w} {h})", quote(text))
}

/// Read `(tick-bang [#:duration secs | #:rate hz])` into a `TickBang`'s
/// `interval` enum field. `#:duration` and `#:rate` are mutually exclusive;
/// neither given yields the default duration.
fn tick_bang_spec(args: &[ExprKind], src: &str) -> Result<Datum, FormatError> {
    let duration = keyword_f64(args, "duration", src)?;
    let rate = keyword_f64(args, "rate", src)?;
    let fields = match (duration, rate) {
        (Some(_), Some(_)) => {
            return Err(FormatError::new(ErrorKind::Malformed(
                "tick-bang: specify #:duration or #:rate, not both".into(),
            )));
        }
        (Some(secs), None) => vec![("interval", tick_interval_datum("Duration", secs))],
        (None, Some(hz)) => vec![("interval", tick_interval_datum("Rate", hz))],
        (None, None) => vec![],
    };
    Ok(node_datum("TickBang", fields))
}

/// The `interval` field datum for a `TickBang` `Interval` enum variant
/// (externally tagged, e.g. `(("Rate" hz))`).
fn tick_interval_datum(variant: &str, value: f64) -> Datum {
    Datum::Map(vec![(variant.to_string(), Datum::F64(value))])
}

/// Write a `TickBang` as a bare `tick-bang` for the default duration, else as
/// `(tick-bang #:duration secs)` or `(tick-bang #:rate hz)` per its unit.
fn write_tick_bang(node: &Datum) -> String {
    match tick_interval(node) {
        Some(("Rate", hz)) => format!("(tick-bang #:rate {hz})"),
        Some(("Duration", secs)) if secs != 1.0 => format!("(tick-bang #:duration {secs})"),
        _ => "tick-bang".to_string(),
    }
}

/// Read a `TickBang`'s `interval` field as `(variant, value)`.
fn tick_interval(node: &Datum) -> Option<(&str, f64)> {
    match node.get("interval")? {
        Datum::Map(entries) => {
            let (variant, value) = entries.first()?;
            Some((variant.as_str(), value.as_f64()?))
        }
        _ => None,
    }
}

/// Write a `Number` as a bare `number` when all config is default, else as
/// `(number #:min m #:max m #:precision n #:no-push-eval)` with only the
/// non-default fields.
fn write_number(node: &Datum) -> String {
    let min = node.get("min").and_then(Datum::as_f64);
    let max = node.get("max").and_then(Datum::as_f64);
    let precision = node.get("precision").and_then(Datum::as_i64);
    let push = node
        .get("push_eval_on_edit")
        .and_then(Datum::as_bool)
        .unwrap_or(true);
    if min.is_none() && max.is_none() && precision.is_none() && push {
        return "number".to_string();
    }
    let mut parts = Vec::new();
    if let Some(min) = min {
        parts.push(format!("#:min {min}"));
    }
    if let Some(max) = max {
        parts.push(format!("#:max {max}"));
    }
    if let Some(precision) = precision {
        parts.push(format!("#:precision {precision}"));
    }
    if !push {
        parts.push("#:no-push-eval".to_string());
    }
    format!("(number {})", parts.join(" "))
}

fn write_log(node: &Datum) -> String {
    match node.get("level").and_then(Datum::as_str) {
        Some(level) if !level.eq_ignore_ascii_case("info") => {
            format!("(log {})", level.to_ascii_lowercase())
        }
        _ => "(log)".to_string(),
    }
}

// -- helpers -----------------------------------------------------------------

fn int_at(e: &ExprKind, src: &str) -> Result<i64, FormatError> {
    sexpr::as_i64(e, src)
        .ok_or_else(|| err_at(e, src, ErrorKind::Malformed("expected an integer".into())))
}

/// Find a `#:<key>` keyword in `args` and return the following integer value.
fn keyword_int(args: &[ExprKind], key: &str, src: &str) -> Result<Option<i64>, FormatError> {
    for (i, a) in args.iter().enumerate() {
        if as_keyword(a).as_deref() == Some(key) {
            let val = args
                .get(i + 1)
                .map(|v| int_at(v, src))
                .transpose()?
                .ok_or_else(|| {
                    err_at(
                        a,
                        src,
                        ErrorKind::Malformed(format!("#:{key} requires an integer")),
                    )
                })?;
            return Ok(Some(val));
        }
    }
    Ok(None)
}

fn float_at(e: &ExprKind, src: &str) -> Result<f64, FormatError> {
    sexpr::as_f64(e, src)
        .ok_or_else(|| err_at(e, src, ErrorKind::Malformed("expected a number".into())))
}

/// Find a `#:<key>` keyword in `args` and return the following float value.
fn keyword_f64(args: &[ExprKind], key: &str, src: &str) -> Result<Option<f64>, FormatError> {
    for (i, a) in args.iter().enumerate() {
        if as_keyword(a).as_deref() == Some(key) {
            let val = args
                .get(i + 1)
                .map(|v| float_at(v, src))
                .transpose()?
                .ok_or_else(|| {
                    err_at(
                        a,
                        src,
                        ErrorKind::Malformed(format!("#:{key} requires a number")),
                    )
                })?;
            return Ok(Some(val));
        }
    }
    Ok(None)
}

/// Whether a bare `#:<key>` flag keyword is present in `args`.
fn has_flag(args: &[ExprKind], key: &str) -> bool {
    args.iter().any(|a| as_keyword(a).as_deref() == Some(key))
}

/// Map a log-level symbol to the `log::Level` serde representation.
fn log_level(sym: &str, e: &ExprKind, src: &str) -> Result<String, FormatError> {
    match sym.to_ascii_lowercase().as_str() {
        "error" => Ok("ERROR".into()),
        "warn" => Ok("WARN".into()),
        "info" => Ok("INFO".into()),
        "debug" => Ok("DEBUG".into()),
        "trace" => Ok("TRACE".into()),
        other => Err(err_at(
            e,
            src,
            ErrorKind::Malformed(format!("unknown log level `{other}`")),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A custom sugar for a hypothetical node set: `(gain <db>)` and bare `mute`.
    struct GainSugar;

    impl Sugar for GainSugar {
        fn read_spec(
            &self,
            head: &str,
            args: &[ExprKind],
            src: &str,
        ) -> Result<Option<Datum>, FormatError> {
            if head != "gain" {
                return Ok(None);
            }
            let db = args
                .first()
                .and_then(|e| sexpr::as_i64(e, src))
                .unwrap_or(0);
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
        sugar.read_spec(&head, &args[1..], text).expect("read_spec")
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
        let (gain, default) = (GainSugar, DefaultSugar);
        let sugars = Sugars(vec![&gain, &default]);

        // The custom keyword is handled by GainSugar.
        let g = read_spec(&sugars, "(gain 3)").expect("gain recognised");
        assert_eq!(g.get("db").and_then(Datum::as_i64), Some(3));

        // A built-in keyword still resolves through the composed DefaultSugar.
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
        assert!(s.read_spec("expr", &[], "").expect("ok").is_none());
        assert!(s.read_bare("inlet").is_none());
        assert!(
            s.write_spec("Inlet", &node_datum("Inlet", vec![]))
                .is_none()
        );
        assert!(s.keyword_for_tag("Inlet").is_none());
    }

    #[test]
    fn inlet_outlet_docs_round_trip() {
        let s = DefaultSugar;

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

    #[test]
    fn number_config_round_trips() {
        let s = DefaultSugar;

        // A bare (default) number stays bare, whether read as a keyword or spec.
        let bare = s.read_bare("number").expect("bare number");
        assert_eq!(s.write_spec("Number", &bare).as_deref(), Some("number"));
        let empty = read_spec(&s, "(number)").expect("empty spec");
        assert_eq!(s.write_spec("Number", &empty).as_deref(), Some("number"));

        // Min/max bounds.
        let mm = read_spec(&s, "(number #:min 0 #:max 100)").expect("min/max");
        assert_eq!(mm.get("min").and_then(Datum::as_f64), Some(0.0));
        assert_eq!(mm.get("max").and_then(Datum::as_f64), Some(100.0));
        assert_eq!(
            s.write_spec("Number", &mm).as_deref(),
            Some("(number #:min 0 #:max 100)"),
        );

        // Display precision.
        let p = read_spec(&s, "(number #:precision 2)").expect("precision");
        assert_eq!(p.get("precision").and_then(Datum::as_i64), Some(2));
        assert_eq!(
            s.write_spec("Number", &p).as_deref(),
            Some("(number #:precision 2)"),
        );

        // Disabled push-eval.
        let np = read_spec(&s, "(number #:no-push-eval)").expect("no push-eval");
        assert_eq!(
            np.get("push_eval_on_edit").and_then(Datum::as_bool),
            Some(false)
        );
        assert_eq!(
            s.write_spec("Number", &np).as_deref(),
            Some("(number #:no-push-eval)"),
        );

        // Everything at once round-trips in canonical order.
        let all = read_spec(
            &s,
            "(number #:min -1.5 #:max 1.5 #:precision 3 #:no-push-eval)",
        )
        .expect("all");
        assert_eq!(
            s.write_spec("Number", &all).as_deref(),
            Some("(number #:min -1.5 #:max 1.5 #:precision 3 #:no-push-eval)"),
        );
    }

    #[test]
    fn tick_bang_config_round_trips() {
        let s = DefaultSugar;

        // A bare (default-duration) tick! stays bare, read as keyword or spec.
        let bare = s.read_bare("tick-bang").expect("bare tick-bang");
        assert_eq!(
            s.write_spec("TickBang", &bare).as_deref(),
            Some("tick-bang")
        );

        // A custom duration round-trips via the `#:duration` keyword (seconds).
        let d = read_spec(&s, "(tick-bang #:duration 0.5)").expect("duration");
        assert_eq!(
            s.write_spec("TickBang", &d).as_deref(),
            Some("(tick-bang #:duration 0.5)"),
        );

        // A rate round-trips via the `#:rate` keyword (Hz), stored exactly.
        let r = read_spec(&s, "(tick-bang #:rate 60)").expect("rate");
        assert_eq!(
            s.write_spec("TickBang", &r).as_deref(),
            Some("(tick-bang #:rate 60)"),
        );

        // An explicit default duration also writes as the bare keyword.
        let one = read_spec(&s, "(tick-bang #:duration 1)").expect("default duration");
        assert_eq!(s.write_spec("TickBang", &one).as_deref(), Some("tick-bang"));

        // `#:duration` and `#:rate` are mutually exclusive.
        let text = "(tick-bang #:duration 0.5 #:rate 60)";
        let exprs = sexpr::read(text).expect("read");
        let args = sexpr::list_args(&exprs[0]).expect("list");
        assert!(s.read_spec("tick-bang", &args[1..], text).is_err());
    }
}
