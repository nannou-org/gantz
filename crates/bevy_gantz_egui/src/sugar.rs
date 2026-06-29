//! `.gantz` keyword sugar for this crate's self-driven node set.
//!
//! [`BevySugar`] provides the keywords for the bevy nodes: bare `update-bang`
//! and `(tick-bang [#:duration secs | #:rate hz])`. The `tick-bang` read/write
//! logic lives with the node in [`crate::node::tick_bang`]. Compose it with
//! [`gantz_format::CoreSugar`] (and the other crates' sugars) via
//! [`gantz_format::Sugars`].

use crate::node::tick_bang;
use gantz_format::{Datum, FormatError, Sugar, SugarArgs, node_datum};

/// Keyword sugar for [`UpdateBang`](crate::node::UpdateBang) and
/// [`TickBang`](crate::node::TickBang).
#[derive(Clone, Copy, Debug, Default)]
pub struct BevySugar;

/// Sugar keyword -> typetag tag, for the bevy builtins that lower to a plain
/// serde object with no extra arguments.
const KEYWORD_TAG: &[(&str, &str)] = &[("update-bang", "UpdateBang"), ("tick-bang", "TickBang")];

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

impl Sugar for BevySugar {
    fn read_spec(&self, head: &str, args: SugarArgs<'_>) -> Result<Option<Datum>, FormatError> {
        let datum = match head {
            "tick-bang" => tick_bang::read_sugar(args)?,
            _ => return Ok(None),
        };
        Ok(Some(datum))
    }

    fn read_bare(&self, keyword: &str) -> Option<Datum> {
        tag_for_keyword(keyword).map(|tag| node_datum(tag, vec![]))
    }

    fn write_spec(&self, tag: &str, node: &Datum) -> Option<String> {
        match tag {
            "TickBang" => Some(tick_bang::write_sugar(node)),
            other => keyword_for_tag(other).map(str::to_string),
        }
    }

    fn keyword_for_tag(&self, tag: &str) -> Option<&str> {
        keyword_for_tag(tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_format::sexpr;

    /// Read a single sugar form's text through `BevySugar`, as the format does.
    fn read_spec(text: &str) -> Option<Datum> {
        let exprs = sexpr::read(text).expect("read");
        let args = sexpr::list_args(&exprs[0]).expect("list");
        let head = sexpr::as_symbol(&args[0]).expect("head");
        BevySugar
            .read_spec(&head, SugarArgs::new(&args[1..], text))
            .expect("read_spec")
    }

    #[test]
    fn update_bang_round_trips() {
        let bare = BevySugar
            .read_bare("update-bang")
            .expect("bare update-bang");
        assert_eq!(bare.get("type").and_then(Datum::as_str), Some("UpdateBang"));
        assert_eq!(
            BevySugar.write_spec("UpdateBang", &bare).as_deref(),
            Some("update-bang")
        );
    }

    #[test]
    fn tick_bang_config_round_trips() {
        let s = BevySugar;

        // A bare (default-duration) tick! stays bare, read as keyword or spec.
        let bare = s.read_bare("tick-bang").expect("bare tick-bang");
        assert_eq!(
            s.write_spec("TickBang", &bare).as_deref(),
            Some("tick-bang")
        );

        // A custom duration round-trips via the `#:duration` keyword (seconds).
        let d = read_spec("(tick-bang #:duration 0.5)").expect("duration");
        assert_eq!(
            s.write_spec("TickBang", &d).as_deref(),
            Some("(tick-bang #:duration 0.5)"),
        );

        // A rate round-trips via the `#:rate` keyword (Hz), stored exactly.
        let r = read_spec("(tick-bang #:rate 60)").expect("rate");
        assert_eq!(
            s.write_spec("TickBang", &r).as_deref(),
            Some("(tick-bang #:rate 60)"),
        );

        // An explicit default duration also writes as the bare keyword.
        let one = read_spec("(tick-bang #:duration 1)").expect("default duration");
        assert_eq!(s.write_spec("TickBang", &one).as_deref(), Some("tick-bang"));

        // `#:duration` and `#:rate` are mutually exclusive.
        let text = "(tick-bang #:duration 0.5 #:rate 60)";
        let exprs = sexpr::read(text).expect("read");
        let args = sexpr::list_args(&exprs[0]).expect("list");
        assert!(
            s.read_spec("tick-bang", SugarArgs::new(&args[1..], text))
                .is_err()
        );
    }
}
