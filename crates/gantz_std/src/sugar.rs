//! `.gantz` keyword sugar for the standard node set.
//!
//! [`StdSugar`] provides the human-friendly keywords for this crate's nodes:
//! bare `bang`, `(number [#:min m] [#:max m] [#:precision n] [#:no-push-eval])`
//! and `(log [level])`. Compose it with [`gantz_format::CoreSugar`] (and the
//! other crates' sugars) via [`gantz_format::Sugars`].

use gantz_format::{Datum, FormatError, Sugar, SugarArgs, node_datum};

/// Keyword sugar for [`Bang`](crate::Bang), [`Number`](crate::Number) and
/// [`Log`](crate::Log).
#[derive(Clone, Copy, Debug, Default)]
pub struct StdSugar;

/// Sugar keyword -> typetag tag, for the std builtins that lower to a plain
/// serde object with no extra arguments.
const KEYWORD_TAG: &[(&str, &str)] = &[("bang", "Bang"), ("number", "Number"), ("log", "Log")];

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

impl Sugar for StdSugar {
    fn read_spec(&self, head: &str, args: SugarArgs<'_>) -> Result<Option<Datum>, FormatError> {
        let datum = match head {
            "number" => number_spec(args)?,
            "log" => log_spec(args)?,
            _ => return Ok(None),
        };
        Ok(Some(datum))
    }

    fn read_bare(&self, keyword: &str) -> Option<Datum> {
        match keyword {
            // A bare `log` defaults to the INFO level (not an empty node).
            "log" => Some(node_datum(
                "Log",
                vec![("level", Datum::Str("INFO".into()))],
            )),
            _ => tag_for_keyword(keyword).map(|tag| node_datum(tag, vec![])),
        }
    }

    fn write_spec(&self, tag: &str, node: &Datum) -> Option<String> {
        match tag {
            "Number" => Some(write_number(node)),
            "Log" => Some(write_log(node)),
            other => keyword_for_tag(other).map(str::to_string),
        }
    }

    fn keyword_for_tag(&self, tag: &str) -> Option<&str> {
        keyword_for_tag(tag)
    }
}

// -- reading -----------------------------------------------------------------

/// Read a `(number [#:min m] [#:max m] [#:precision n] [#:no-push-eval])` form.
/// Only the non-default fields are emitted, so a bare `number` stays bare.
fn number_spec(args: SugarArgs<'_>) -> Result<Datum, FormatError> {
    let mut fields = Vec::new();
    if let Some(min) = args.keyword_f64("min")? {
        fields.push(("min", Datum::F64(min)));
    }
    if let Some(max) = args.keyword_f64("max")? {
        fields.push(("max", Datum::F64(max)));
    }
    if let Some(precision) = args.keyword_int("precision")? {
        fields.push(("precision", Datum::U64(precision.max(0) as u64)));
    }
    if args.has_flag("no-push-eval") {
        fields.push(("push_eval_on_edit", Datum::Bool(false)));
    }
    Ok(node_datum("Number", fields))
}

/// Read a `(log [level])` form, mapping the level symbol to the serde string.
fn log_spec(args: SugarArgs<'_>) -> Result<Datum, FormatError> {
    let level = match args.symbol_at(0) {
        Some(sym) => log_level(&sym)
            .ok_or_else(|| args.malformed_at(0, format!("unknown log level `{sym}`")))?,
        None => "INFO".to_string(),
    };
    Ok(node_datum("Log", vec![("level", Datum::Str(level))]))
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

// -- writing -----------------------------------------------------------------

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

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_format::sexpr;

    /// Read a single sugar form's text through `StdSugar`, as the format does.
    fn read_spec(text: &str) -> Option<Datum> {
        let exprs = sexpr::read(text).expect("read");
        let args = sexpr::list_args(&exprs[0]).expect("list");
        let head = sexpr::as_symbol(&args[0]).expect("head");
        StdSugar
            .read_spec(&head, SugarArgs::new(&args[1..], text))
            .expect("read_spec")
    }

    #[test]
    fn bang_round_trips() {
        let bare = StdSugar.read_bare("bang").expect("bare bang");
        assert_eq!(bare.get("type").and_then(Datum::as_str), Some("Bang"));
        assert_eq!(StdSugar.write_spec("Bang", &bare).as_deref(), Some("bang"));
    }

    #[test]
    fn number_config_round_trips() {
        let s = StdSugar;

        // A bare (default) number stays bare, whether read as a keyword or spec.
        let bare = s.read_bare("number").expect("bare number");
        assert_eq!(s.write_spec("Number", &bare).as_deref(), Some("number"));
        let empty = read_spec("(number)").expect("empty spec");
        assert_eq!(s.write_spec("Number", &empty).as_deref(), Some("number"));

        // Min/max bounds.
        let mm = read_spec("(number #:min 0 #:max 100)").expect("min/max");
        assert_eq!(mm.get("min").and_then(Datum::as_f64), Some(0.0));
        assert_eq!(mm.get("max").and_then(Datum::as_f64), Some(100.0));
        assert_eq!(
            s.write_spec("Number", &mm).as_deref(),
            Some("(number #:min 0 #:max 100)"),
        );

        // Display precision.
        let p = read_spec("(number #:precision 2)").expect("precision");
        assert_eq!(p.get("precision").and_then(Datum::as_i64), Some(2));
        assert_eq!(
            s.write_spec("Number", &p).as_deref(),
            Some("(number #:precision 2)"),
        );

        // Disabled push-eval.
        let np = read_spec("(number #:no-push-eval)").expect("no push-eval");
        assert_eq!(
            np.get("push_eval_on_edit").and_then(Datum::as_bool),
            Some(false)
        );
        assert_eq!(
            s.write_spec("Number", &np).as_deref(),
            Some("(number #:no-push-eval)"),
        );

        // Everything at once round-trips in canonical order.
        let all =
            read_spec("(number #:min -1.5 #:max 1.5 #:precision 3 #:no-push-eval)").expect("all");
        assert_eq!(
            s.write_spec("Number", &all).as_deref(),
            Some("(number #:min -1.5 #:max 1.5 #:precision 3 #:no-push-eval)"),
        );
    }

    #[test]
    fn log_level_round_trips() {
        let s = StdSugar;

        // A bare `log` and an explicit `(log)` both default to INFO and write bare.
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
