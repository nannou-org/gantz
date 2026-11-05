//! `.gantz` keyword sugar for the base nodes that are defined as data.
//!
//! [`BaseSugar`] provides `(get "key")`, `(set "key")` and
//! `(literal "text")`. Compose it with the other crates' sugars via
//! [`gantz_format::Sugars`].

use crate::{Get, Literal, Set};
use gantz_format::{Datum, FormatError, Sugar, SugarArgs, node_datum};
use gantz_nodedef::NodeDef;

/// Keyword sugar for [`Get`], [`Set`] and [`Literal`].
#[derive(Clone, Copy, Debug, Default)]
pub struct BaseSugar;

/// The [`Get`] keyword, also its builtin name.
pub const GET: &str = "get";

/// The [`Set`] keyword, also its builtin name.
pub const SET: &str = "set";

/// The [`Literal`] keyword, also its builtin name.
pub const LITERAL: &str = "literal";

/// Each node with one text field, as its keyword, its tag and the name of
/// the field.
const NODES: &[(&str, &str, &str)] = &[
    (GET, Get::TAG, "key"),
    (SET, Set::TAG, "key"),
    (LITERAL, Literal::TAG, "src"),
];

impl Sugar for BaseSugar {
    fn read_spec(&self, head: &str, args: SugarArgs<'_>) -> Result<Option<Datum>, FormatError> {
        Ok(by_keyword(head).map(|(tag, field)| text_datum(tag, field, args.str_at(0))))
    }

    fn read_bare(&self, keyword: &str) -> Option<Datum> {
        by_keyword(keyword).map(|(tag, field)| text_datum(tag, field, None))
    }

    fn write_spec(&self, tag: &str, node: &Datum) -> Option<String> {
        let (keyword, _, field) = NODES.iter().find(|(_, t, _)| *t == tag)?;
        let text = node.get(field).and_then(Datum::as_str).unwrap_or("");
        Some(format!("({keyword} {text:?})"))
    }

    fn keyword_for_tag(&self, tag: &str) -> Option<&str> {
        NODES
            .iter()
            .find(|(_, t, _)| *t == tag)
            .map(|(keyword, _, _)| *keyword)
    }
}

/// The tag and the field of the node with the given keyword.
fn by_keyword(keyword: &str) -> Option<(&'static str, &'static str)> {
    NODES
        .iter()
        .find(|(k, _, _)| *k == keyword)
        .map(|(_, tag, field)| (*tag, *field))
}

/// The stored form of a node with one text field.
fn text_datum(tag: &str, field: &str, text: Option<String>) -> Datum {
    node_datum(tag, vec![(field, Datum::Str(text.unwrap_or_default()))])
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_format::sexpr;

    fn read(text: &str) -> Datum {
        let exprs = sexpr::read(text).expect("read");
        let args = sexpr::list_args(&exprs[0]).expect("list");
        let head = sexpr::as_symbol(&args[0]).expect("head");
        BaseSugar
            .read_spec(&head, SugarArgs::new(&args[1..], text))
            .expect("read_spec")
            .expect("recognised")
    }

    // Each node round-trips through read and write, escaping quotes in its
    // text, and reads bare as empty text.
    #[test]
    fn nodes_round_trip() {
        for &(keyword, tag, field) in NODES {
            let text = format!(r#"({keyword} "(a \"b\")")"#);
            let d = read(&text);
            assert_eq!(d.get(field).and_then(Datum::as_str), Some(r#"(a "b")"#));
            assert_eq!(BaseSugar.write_spec(tag, &d), Some(text));
            assert_eq!(BaseSugar.keyword_for_tag(tag), Some(keyword));
            let empty = node_datum(tag, vec![(field, Datum::Str(String::new()))]);
            assert_eq!(BaseSugar.read_bare(keyword), Some(empty));
        }
    }
}
