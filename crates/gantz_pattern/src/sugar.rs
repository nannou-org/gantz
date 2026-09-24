//! `.gantz` keyword sugar for the pattern node set.
//!
//! [`PatternSugar`] provides `(pmini "notation")` and, with the `egui`
//! feature, `(pplot <keyword>...)`. Compose it with the other
//! crates' sugars via [`gantz_format::Sugars`].

use crate::Pmini;
#[cfg(feature = "egui")]
use crate::egui::pplot::sugar as pplot;
use gantz_format::{Datum, FormatError, Sugar, SugarArgs, node_datum};
use gantz_nodetag::NodeTag;

/// Keyword sugar for [`Pmini`] and, with the `egui` feature, `Pplot`.
#[derive(Clone, Copy, Debug, Default)]
pub struct PatternSugar;

impl Sugar for PatternSugar {
    fn read_spec(&self, head: &str, args: SugarArgs<'_>) -> Result<Option<Datum>, FormatError> {
        match head {
            "pmini" => {
                let src = args.str_at(0).unwrap_or_default();
                Ok(Some(node_datum(Pmini::TAG, vec![("src", Datum::Str(src))])))
            }
            #[cfg(feature = "egui")]
            pplot::KEYWORD => pplot::read_spec(args).map(Some),
            _ => Ok(None),
        }
    }

    fn read_bare(&self, keyword: &str) -> Option<Datum> {
        match keyword {
            "pmini" => Some(node_datum(
                Pmini::TAG,
                vec![("src", Datum::Str(String::new()))],
            )),
            #[cfg(feature = "egui")]
            pplot::KEYWORD => pplot::read_bare(),
            _ => None,
        }
    }

    fn write_spec(&self, tag: &str, node: &Datum) -> Option<String> {
        if tag == Pmini::TAG {
            let src = node.get("src").and_then(Datum::as_str).unwrap_or("");
            return Some(format!("(pmini {src:?})"));
        }
        #[cfg(feature = "egui")]
        if tag == pplot::tag() {
            return pplot::write_spec(node);
        }
        None
    }

    fn keyword_for_tag(&self, tag: &str) -> Option<&str> {
        if tag == Pmini::TAG {
            return Some("pmini");
        }
        #[cfg(feature = "egui")]
        if tag == pplot::tag() {
            return Some(pplot::KEYWORD);
        }
        None
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

    // `(pmini "notation")` round-trips through read and write, escaping
    // quotes in the notation.
    #[test]
    fn pm_round_trips() {
        let d = read(r#"(pmini "bd(3,8) ~ [sn sn]")"#);
        assert_eq!(
            d.get("src").and_then(Datum::as_str),
            Some("bd(3,8) ~ [sn sn]")
        );
        assert_eq!(
            PatternSugar.write_spec(Pmini::TAG, &d).as_deref(),
            Some(r#"(pmini "bd(3,8) ~ [sn sn]")"#),
        );
        // Bare `pmini` reads as an empty notation.
        assert_eq!(
            PatternSugar.read_bare("pmini"),
            Some(node_datum(
                Pmini::TAG,
                vec![("src", Datum::Str(String::new()))]
            )),
        );
        assert_eq!(PatternSugar.keyword_for_tag(Pmini::TAG), Some("pmini"));
    }
}
