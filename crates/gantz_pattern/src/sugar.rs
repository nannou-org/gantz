//! `.gantz` keyword sugar for the pattern node set.
//!
//! [`PatternSugar`] provides `(pm "notation")`. Compose it with the other
//! crates' sugars via [`gantz_format::Sugars`].

use crate::Pm;
use gantz_format::{Datum, FormatError, Sugar, SugarArgs, node_datum};
use gantz_nodetag::NodeTag;

/// Keyword sugar for [`Pm`].
#[derive(Clone, Copy, Debug, Default)]
pub struct PatternSugar;

impl Sugar for PatternSugar {
    fn read_spec(&self, head: &str, args: SugarArgs<'_>) -> Result<Option<Datum>, FormatError> {
        if head != "pm" {
            return Ok(None);
        }
        let src = args.str_at(0).unwrap_or_default();
        Ok(Some(node_datum(Pm::TAG, vec![("src", Datum::Str(src))])))
    }

    fn read_bare(&self, keyword: &str) -> Option<Datum> {
        (keyword == "pm").then(|| node_datum(Pm::TAG, vec![("src", Datum::Str(String::new()))]))
    }

    fn write_spec(&self, tag: &str, node: &Datum) -> Option<String> {
        if tag != Pm::TAG {
            return None;
        }
        let src = node.get("src").and_then(Datum::as_str).unwrap_or("");
        Some(format!("(pm {src:?})"))
    }

    fn keyword_for_tag(&self, tag: &str) -> Option<&str> {
        (tag == Pm::TAG).then_some("pm")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_format::sexpr;

    fn read(text: &str) -> Datum {
        let exprs = sexpr::read(text).expect("read");
        let args = sexpr::list_args(&exprs[0]).expect("list");
        let head = sexpr::as_symbol(&args[0]).expect("head");
        PatternSugar
            .read_spec(&head, SugarArgs::new(&args[1..], text))
            .expect("read_spec")
            .expect("recognised")
    }

    // `(pm "notation")` round-trips through read and write, escaping
    // quotes in the notation.
    #[test]
    fn pm_round_trips() {
        let d = read(r#"(pm "bd(3,8) ~ [sn sn]")"#);
        assert_eq!(
            d.get("src").and_then(Datum::as_str),
            Some("bd(3,8) ~ [sn sn]")
        );
        assert_eq!(
            PatternSugar.write_spec(Pm::TAG, &d).as_deref(),
            Some(r#"(pm "bd(3,8) ~ [sn sn]")"#),
        );
        // Bare `pm` reads as an empty notation.
        assert_eq!(
            PatternSugar.read_bare("pm"),
            Some(node_datum(
                Pm::TAG,
                vec![("src", Datum::Str(String::new()))]
            )),
        );
        assert_eq!(PatternSugar.keyword_for_tag(Pm::TAG), Some("pm"));
    }
}
