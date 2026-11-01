//! `.gantz` keyword sugar for the pattern node set.
//!
//! [`PatternSugar`] provides `(pat.mini "notation")`, `(pat.key "key")`,
//! `(pat.pick "key")` and `(pat.plot <keyword>...)`. Compose it with the
//! other crates' sugars via [`gantz_format::Sugars`]. The keywords from
//! before the `pat.` namespace, `pmini` and `pplot`, still read.

use crate::plot as pplot;
use crate::{Pkey, Pmini, Ppick, Pplot};
use gantz_format::{Datum, FormatError, Sugar, SugarArgs, node_datum};
use gantz_nodedef::NodeDef;

/// Keyword sugar for [`Pmini`], [`Pkey`], [`Ppick`] and [`Pplot`].
#[derive(Clone, Copy, Debug, Default)]
pub struct PatternSugar;

/// The [`Pmini`] keyword, also its builtin name.
pub const MINI: &str = "pat.mini";

/// The [`Pkey`] keyword, also its builtin name.
pub const KEY: &str = "pat.key";

/// The [`Ppick`] keyword, also its builtin name.
pub const PICK: &str = "pat.pick";

/// The [`Pmini`] keyword from before the `pat.` namespace.
const OLD_MINI: &str = "pmini";

impl Sugar for PatternSugar {
    fn read_spec(&self, head: &str, args: SugarArgs<'_>) -> Result<Option<Datum>, FormatError> {
        match head {
            MINI | OLD_MINI => {
                let src = args.str_at(0).unwrap_or_default();
                Ok(Some(node_datum(Pmini::TAG, vec![("src", Datum::Str(src))])))
            }
            KEY => Ok(Some(key_datum(Pkey::TAG, args.str_at(0)))),
            PICK => Ok(Some(key_datum(Ppick::TAG, args.str_at(0)))),
            pplot::KEYWORD | pplot::OLD_KEYWORD => pplot::read_spec(args).map(Some),
            _ => Ok(None),
        }
    }

    fn read_bare(&self, keyword: &str) -> Option<Datum> {
        match keyword {
            MINI | OLD_MINI => Some(node_datum(
                Pmini::TAG,
                vec![("src", Datum::Str(String::new()))],
            )),
            KEY => Some(key_datum(Pkey::TAG, None)),
            PICK => Some(key_datum(Ppick::TAG, None)),
            pplot::KEYWORD | pplot::OLD_KEYWORD => pplot::read_bare(),
            _ => None,
        }
    }

    fn write_spec(&self, tag: &str, node: &Datum) -> Option<String> {
        if tag == Pmini::TAG {
            let src = node.get("src").and_then(Datum::as_str).unwrap_or("");
            return Some(format!("({MINI} {src:?})"));
        }
        if tag == Pkey::TAG || tag == Ppick::TAG {
            let keyword = self.keyword_for_tag(tag)?;
            let key = node.get("key").and_then(Datum::as_str).unwrap_or("");
            return Some(format!("({keyword} {key:?})"));
        }
        if tag == Pplot::TAG {
            return pplot::write_spec(node);
        }
        None
    }

    fn keyword_for_tag(&self, tag: &str) -> Option<&str> {
        if tag == Pmini::TAG {
            return Some(MINI);
        }
        if tag == Pkey::TAG {
            return Some(KEY);
        }
        if tag == Ppick::TAG {
            return Some(PICK);
        }
        if tag == Pplot::TAG {
            return Some(pplot::KEYWORD);
        }
        None
    }
}

/// The stored form of a keyed node.
fn key_datum(tag: &str, key: Option<String>) -> Datum {
    let key = Datum::Str(key.unwrap_or_default());
    node_datum(tag, vec![("key", key)])
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

    // `(pat.mini "notation")` round-trips through read and write, escaping
    // quotes in the notation.
    #[test]
    fn pm_round_trips() {
        let d = read(r#"(pat.mini "bd(3,8) ~ [sn sn]")"#);
        assert_eq!(
            d.get("src").and_then(Datum::as_str),
            Some("bd(3,8) ~ [sn sn]")
        );
        assert_eq!(
            PatternSugar.write_spec(Pmini::TAG, &d).as_deref(),
            Some(r#"(pat.mini "bd(3,8) ~ [sn sn]")"#),
        );
        // Bare `pat.mini` reads as an empty notation.
        let empty = node_datum(Pmini::TAG, vec![("src", Datum::Str(String::new()))]);
        assert_eq!(PatternSugar.read_bare("pat.mini"), Some(empty.clone()));
        assert_eq!(PatternSugar.keyword_for_tag(Pmini::TAG), Some("pat.mini"));
        // The old keyword still reads.
        assert_eq!(read(r#"(pmini "bd(3,8) ~ [sn sn]")"#), d);
        assert_eq!(PatternSugar.read_bare("pmini"), Some(empty));
    }

    // The keyed nodes round-trip through read and write, and read bare as
    // an empty key.
    #[test]
    fn keyed_nodes_round_trip() {
        for (keyword, tag) in [(KEY, Pkey::TAG), (PICK, Ppick::TAG)] {
            let text = format!(r#"({keyword} "freq")"#);
            let d = read(&text);
            assert_eq!(d.get("key").and_then(Datum::as_str), Some("freq"));
            assert_eq!(PatternSugar.write_spec(tag, &d), Some(text));
            assert_eq!(PatternSugar.keyword_for_tag(tag), Some(keyword));
            let empty = node_datum(tag, vec![("key", Datum::Str(String::new()))]);
            assert_eq!(PatternSugar.read_bare(keyword), Some(empty));
        }
    }
}
