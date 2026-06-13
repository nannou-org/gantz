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

use crate::datum::{Datum, datum_field, datum_int, datum_seq, datum_str};
use crate::error::{ErrorKind, FormatError};
use crate::sexpr::{self, as_keyword, as_string, as_symbol, quote, span_src};
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
    ("add", "Add"),
    ("inspect", "Inspect"),
    ("frame-bang", "FrameBang"),
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
            "expr" => expr_spec(args, src)?,
            "branch" => branch_spec(args, src)?,
            "comment" => comment_spec(args, src)?,
            "number" => node_datum("Number", vec![]),
            "log" => log_spec(args, src)?,
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
            "Expr" => Some(write_expr(node)),
            "Branch" => Some(write_branch(node)),
            "Comment" => Some(write_comment(node)),
            "Log" => Some(write_log(node)),
            other => keyword_for_tag(other).map(str::to_string),
        }
    }

    fn keyword_for_tag(&self, tag: &str) -> Option<&str> {
        keyword_for_tag(tag)
    }
}

// -- built-in reading --------------------------------------------------------

/// Build a node datum from a typetag tag and ordered fields (the `type` field
/// is prepended).
fn node_datum(tag: &str, fields: Vec<(&str, Datum)>) -> Datum {
    let mut entries = Vec::with_capacity(fields.len() + 1);
    entries.push(("type".to_string(), Datum::Str(tag.to_string())));
    entries.extend(fields.into_iter().map(|(k, v)| (k.to_string(), v)));
    Datum::Map(entries)
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

fn log_spec(args: &[ExprKind], src: &str) -> Result<Datum, FormatError> {
    let level = match args.first().and_then(as_symbol) {
        Some(s) => log_level(&s, &args[0], src)?,
        None => "INFO".to_string(),
    };
    Ok(node_datum("Log", vec![("level", Datum::Str(level))]))
}

// -- built-in writing --------------------------------------------------------

fn write_expr(node: &Datum) -> String {
    let src = datum_field(node, "src")
        .and_then(datum_str)
        .unwrap_or("'()");
    match datum_field(node, "outputs").and_then(datum_int) {
        Some(n) if n != 1 => format!("(expr {src} #:out {n})"),
        _ => format!("(expr {src})"),
    }
}

fn write_branch(node: &Datum) -> String {
    let src = datum_field(node, "src")
        .and_then(datum_str)
        .unwrap_or("'()");
    let masks = datum_field(node, "branches")
        .and_then(datum_seq)
        .map(|a| {
            a.iter()
                .filter_map(datum_str)
                .map(quote)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    format!("(branch {src} {masks})")
}

fn write_comment(node: &Datum) -> String {
    let text = datum_field(node, "text").and_then(datum_str).unwrap_or("");
    let (w, h) = datum_field(node, "size")
        .and_then(datum_seq)
        .and_then(|a| Some((datum_int(a.first()?)?, datum_int(a.get(1)?)?)))
        .unwrap_or((100, 40));
    format!("(comment {} {w} {h})", quote(text))
}

fn write_log(node: &Datum) -> String {
    match datum_field(node, "level").and_then(datum_str) {
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

fn err_at(e: &ExprKind, src: &str, kind: ErrorKind) -> FormatError {
    FormatError::new(kind).at(sexpr::span(e).unwrap_or_default(), src)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datum::datum_int;

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
                    let db = datum_field(node, "db").and_then(datum_int).unwrap_or(0);
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
        assert_eq!(
            datum_field(&datum, "type").and_then(datum_str),
            Some("Gain")
        );
        assert_eq!(datum_field(&datum, "db").and_then(datum_int), Some(6));
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
        assert_eq!(datum_field(&g, "db").and_then(datum_int), Some(3));

        // A built-in keyword still resolves through the composed DefaultSugar.
        let e = read_spec(&sugars, "(expr (+ 1 2))").expect("expr recognised");
        assert_eq!(datum_field(&e, "type").and_then(datum_str), Some("Expr"));

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
}
