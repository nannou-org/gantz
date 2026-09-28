//! `.gantz` keyword sugar for the io node set.
//!
//! [`IoSugar`] provides the keywords for this crate's nodes. Those are
//! `(log [level])` and a bare `main!`. Compose it with
//! [`gantz_format::CoreSugar`] and the other crates' sugars via
//! [`gantz_format::Sugars`].

use crate::{Log, MainBang};
use gantz_format::{Datum, FormatError, Sugar, SugarArgs, node_datum};
use gantz_nodetag::NodeTag;

/// Keyword sugar for [`Log`] and [`MainBang`].
#[derive(Clone, Copy, Debug, Default)]
pub struct IoSugar;

impl Sugar for IoSugar {
    fn read_spec(&self, head: &str, args: SugarArgs<'_>) -> Result<Option<Datum>, FormatError> {
        let datum = match head {
            "log" => log_spec(args)?,
            _ => return Ok(None),
        };
        Ok(Some(datum))
    }

    fn read_bare(&self, keyword: &str) -> Option<Datum> {
        match keyword {
            // A bare `log` defaults to the INFO level, not an empty node.
            "log" => Some(node_datum(
                Log::TAG,
                vec![("level", Datum::Str("INFO".into()))],
            )),
            "main!" => Some(node_datum(MainBang::TAG, vec![])),
            _ => None,
        }
    }

    fn write_spec(&self, tag: &str, node: &Datum) -> Option<String> {
        match tag {
            Log::TAG => Some(write_log(node)),
            MainBang::TAG => Some("main!".to_string()),
            _ => None,
        }
    }

    fn keyword_for_tag(&self, tag: &str) -> Option<&str> {
        match tag {
            Log::TAG => Some("log"),
            MainBang::TAG => Some("main!"),
            _ => None,
        }
    }
}

/// Read a `(log [level])` form, mapping the level symbol to the serde string.
fn log_spec(args: SugarArgs<'_>) -> Result<Datum, FormatError> {
    let level = match args.symbol_at(0) {
        Some(sym) => log_level(&sym)
            .ok_or_else(|| args.malformed_at(0, format!("unknown log level `{sym}`")))?,
        None => "INFO".to_string(),
    };
    Ok(node_datum(Log::TAG, vec![("level", Datum::Str(level))]))
}

/// Map a log-level symbol to the `log::Level` serde representation.
fn log_level(sym: &str) -> Option<String> {
    match sym.to_ascii_lowercase().as_str() {
        "error" => Some("ERROR".into()),
        "warn" => Some("WARN".into()),
        "info" => Some("INFO".into()),
        "debug" => Some("DEBUG".into()),
        "trace" => Some("TRACE".into()),
        _ => None,
    }
}

fn write_log(node: &Datum) -> String {
    match node.get("level").and_then(Datum::as_str) {
        Some(level) if !level.eq_ignore_ascii_case("info") => {
            format!("(log {})", level.to_ascii_lowercase())
        }
        _ => "(log)".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_format::sexpr;

    /// Read a single sugar form's text through `IoSugar`, as the format does.
    fn read_spec(text: &str) -> Option<Datum> {
        let exprs = sexpr::read(text).expect("read");
        let args = sexpr::list_args(&exprs[0]).expect("list");
        let head = sexpr::as_symbol(&args[0]).expect("head");
        IoSugar
            .read_spec(&head, SugarArgs::new(&args[1..], text))
            .expect("read_spec")
    }

    #[test]
    fn main_bang_round_trips() {
        let bare = IoSugar.read_bare("main!").expect("bare main!");
        assert_eq!(bare.get("type").and_then(Datum::as_str), Some("MainBang"));
        assert_eq!(
            IoSugar.write_spec("MainBang", &bare).as_deref(),
            Some("main!")
        );
        assert_eq!(IoSugar.keyword_for_tag("MainBang"), Some("main!"));
    }

    #[test]
    fn log_level_round_trips() {
        let s = IoSugar;

        // A bare `log` and an explicit `(log)` both default to INFO and write
        // `(log)`.
        let bare = s.read_bare("log").expect("bare log");
        assert_eq!(bare.get("level").and_then(Datum::as_str), Some("INFO"));
        assert_eq!(s.write_spec("Log", &bare).as_deref(), Some("(log)"));
        let info = read_spec("(log info)").expect("info");
        assert_eq!(s.write_spec("Log", &info).as_deref(), Some("(log)"));

        // A non-default level round-trips lower-cased.
        let warn = read_spec("(log warn)").expect("warn");
        assert_eq!(warn.get("level").and_then(Datum::as_str), Some("WARN"));
        assert_eq!(s.write_spec("Log", &warn).as_deref(), Some("(log warn)"));

        // An unknown level errors.
        let exprs = sexpr::read("(log bogus)").expect("read");
        let args = sexpr::list_args(&exprs[0]).expect("list");
        assert!(
            s.read_spec("log", SugarArgs::new(&args[1..], "(log bogus)"))
                .is_err()
        );
    }
}
