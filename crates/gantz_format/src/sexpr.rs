//! A small S-expression toolkit shared by the format and its extenders.
//!
//! The document is read with Steel's own reader
//! ([`steel::parser::parser::Parser::parse_without_lowering`]) so tokenisation
//! (strings, keywords, numbers, identifiers like `->` and `$l`, and arbitrary
//! embedded Steel) matches Steel exactly and special forms stay plain lists.
//! Numbers are read from their verbatim source slice (via the datum's span), so
//! callers that parse [`Form`](crate::Form) text pass that form's own `raw` as
//! `src`.

use crate::error::{ErrorKind, FormatError, Span};
pub use steel::parser::ast::ExprKind;
use steel::parser::parser::Parser;
use steel::parser::tokens::TokenType;

/// Read source text into top-level datums.
pub fn read(text: &str) -> Result<Vec<ExprKind>, FormatError> {
    Parser::parse_without_lowering(text)
        .map_err(|e| FormatError::new(ErrorKind::Read(e.to_string())))
}

/// The elements of a list datum, or `None` if it is not a list.
pub fn list_args(e: &ExprKind) -> Option<&[ExprKind]> {
    match e {
        ExprKind::List(list) => Some(&list.args),
        _ => None,
    }
}

/// The identifier of a symbol datum.
pub fn as_symbol(e: &ExprKind) -> Option<String> {
    match e {
        ExprKind::Atom(a) => match &a.syn.ty {
            TokenType::Identifier(s) => Some(s.resolve().to_string()),
            _ => None,
        },
        _ => None,
    }
}

/// The contents of a string-literal datum.
pub fn as_string(e: &ExprKind) -> Option<String> {
    match e {
        ExprKind::Atom(a) => match &a.syn.ty {
            TokenType::StringLiteral(s) => Some(s.to_string()),
            _ => None,
        },
        _ => None,
    }
}

/// The name of a `#:keyword` datum (without the leading `#:`).
pub fn as_keyword(e: &ExprKind) -> Option<String> {
    match e {
        ExprKind::Atom(a) => match &a.syn.ty {
            TokenType::Keyword(s) => Some(s.resolve().trim_start_matches("#:").to_string()),
            _ => None,
        },
        _ => None,
    }
}

/// The source span of a datum.
pub fn span(e: &ExprKind) -> Option<Span> {
    // steel 0.8's `ExprKind::span` returns a plain `Span`; every datum we read
    // comes from real source text, so it is always present.
    let s = e.span();
    Some(Span::new(s.start as usize, s.end as usize))
}

/// The verbatim source slice a datum covers within `src`.
pub fn span_src<'a>(e: &ExprKind, src: &'a str) -> Option<&'a str> {
    let s = e.span();
    src.get(s.start as usize..s.end as usize)
}

/// Build a [`FormatError`] of `kind` located at the datum's source span.
pub(crate) fn err_at(e: &ExprKind, src: &str, kind: ErrorKind) -> FormatError {
    FormatError::new(kind).at(span(e).unwrap_or_default(), src)
}

/// A numeric datum parsed as an integer (from its verbatim source).
pub fn as_i64(e: &ExprKind, src: &str) -> Option<i64> {
    span_src(e, src)?.parse().ok()
}

/// A numeric datum parsed as a float (from its verbatim source).
pub fn as_f32(e: &ExprKind, src: &str) -> Option<f32> {
    span_src(e, src)?.parse().ok()
}

/// A numeric datum parsed as a double (from its verbatim source).
pub fn as_f64(e: &ExprKind, src: &str) -> Option<f64> {
    span_src(e, src)?.parse().ok()
}

/// Quote a string as a Steel string literal.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// Format a float without scientific notation, using the shortest
/// round-tripping representation.
pub fn num(x: f32) -> String {
    let s = format!("{x}");
    if s.contains('e') || s.contains('E') {
        format!("{x:.6}")
    } else {
        s
    }
}
