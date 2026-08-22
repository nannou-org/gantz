//! `.gantz` keyword sugar for this crate's GUI node set.
//!
//! [`EguiSugar`] provides the keywords for the egui nodes: `(comment <text> [w
//! h])`, `(gui [<role>] [#:display <d>])` and bare `inspect`/`gui`. Compose it
//! with [`gantz_format::CoreSugar`] (and the other crates' sugars) via
//! [`gantz_format::Sugars`].

use crate::node::{Comment, Gui, GuiDisplay, GuiRole, Inspect};
use gantz_format::sexpr::quote;
use gantz_format::{Datum, FormatError, Sugar, SugarArgs, node_datum};
use gantz_nodetag::NodeTag;

/// Keyword sugar for [`Comment`], [`Gui`] and [`Inspect`].
#[derive(Clone, Copy, Debug, Default)]
pub struct EguiSugar;

/// Sugar keyword -> node tag, for the egui builtins whose bare keyword lowers
/// to a default node. Non-default `Comment`/`Gui` forms are handled by
/// explicit `read_spec`/`write_spec` arms.
const KEYWORD_TAG: &[(&str, &str)] = &[
    ("inspect", Inspect::TAG),
    ("comment", Comment::TAG),
    ("gui", Gui::TAG),
];

/// The node tag for a sugar keyword.
fn tag_for_keyword(kw: &str) -> Option<&'static str> {
    KEYWORD_TAG
        .iter()
        .find(|(k, _)| *k == kw)
        .map(|&(_, tag)| tag)
}

/// The sugar keyword for a node tag, if one exists.
fn keyword_for_tag(tag: &str) -> Option<&'static str> {
    KEYWORD_TAG
        .iter()
        .find(|(_, t)| *t == tag)
        .map(|&(kw, _)| kw)
}

impl Sugar for EguiSugar {
    fn read_spec(&self, head: &str, args: SugarArgs<'_>) -> Result<Option<Datum>, FormatError> {
        let datum = match head {
            "comment" => comment_spec(args)?,
            "gui" => gui_spec(args)?,
            _ => return Ok(None),
        };
        Ok(Some(datum))
    }

    fn read_bare(&self, keyword: &str) -> Option<Datum> {
        tag_for_keyword(keyword).map(|tag| node_datum(tag, vec![]))
    }

    fn write_spec(&self, tag: &str, node: &Datum) -> Option<String> {
        match tag {
            "Comment" => Some(write_comment(node)),
            // Must precede the bare-keyword fallback: a non-default `Gui`
            // written as bare `gui` would silently drop its role/display.
            "Gui" => Some(write_gui(node)),
            other => keyword_for_tag(other).map(str::to_string),
        }
    }

    fn keyword_for_tag(&self, tag: &str) -> Option<&str> {
        keyword_for_tag(tag)
    }
}

/// Read a `(comment <text> [w h])` form: required text plus an optional `[w h]`
/// size, defaulting to `[100 40]`.
fn comment_spec(args: SugarArgs<'_>) -> Result<Datum, FormatError> {
    let text = args
        .str_at(0)
        .ok_or_else(|| FormatError::malformed("comment requires text"))?;
    let [w, h] = match (args.int_at(1)?, args.int_at(2)?) {
        (Some(w), Some(h)) => [w.max(0) as u64, h.max(0) as u64],
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

fn write_comment(node: &Datum) -> String {
    let text = node.get("text").and_then(Datum::as_str).unwrap_or("");
    let (w, h) = node
        .get("size")
        .and_then(Datum::as_seq)
        .and_then(|a| Some((a.first()?.as_i64()?, a.get(1)?.as_i64()?)))
        .unwrap_or((100, 40));
    format!("(comment {} {w} {h})", quote(text))
}

/// Read a `(gui [<role>] [#:display <d>])` form: optional positional role
/// symbol (default `body`) and optional display keyword (default `full`).
fn gui_spec(args: SugarArgs<'_>) -> Result<Datum, FormatError> {
    let role = match args.symbol_at(0) {
        Some(s) => GuiRole::from_str(&s)
            .ok_or_else(|| args.malformed_at(0, format!("unknown gui role `{s}`")))?,
        None => GuiRole::default(),
    };
    let display = match args.keyword_symbol("display")? {
        Some(s) => GuiDisplay::from_str(&s)
            .ok_or_else(|| FormatError::malformed(format!("unknown gui display `{s}`")))?,
        None => GuiDisplay::default(),
    };
    Ok(node_datum(
        "Gui",
        vec![
            ("role", Datum::Str(role.as_str().to_string())),
            ("display", Datum::Str(display.as_str().to_string())),
        ],
    ))
}

/// Write the canonical `gui` form: bare when all-default, `(gui <role>)` when
/// only the role differs, and the role always written (even `body`) when a
/// display follows so the positional slot stays unambiguous.
fn write_gui(node: &Datum) -> String {
    let default_role = GuiRole::default().as_str();
    let default_display = GuiDisplay::default().as_str();
    let role = node
        .get("role")
        .and_then(Datum::as_str)
        .unwrap_or(default_role);
    let display = node
        .get("display")
        .and_then(Datum::as_str)
        .unwrap_or(default_display);
    match (role == default_role, display == default_display) {
        (true, true) => "gui".to_string(),
        (_, true) => format!("(gui {role})"),
        (_, false) => format!("(gui {role} #:display {display})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_format::sexpr;

    /// Read a single sugar form's text through `EguiSugar`, as the format does.
    fn read_spec(text: &str) -> Option<Datum> {
        let exprs = sexpr::read(text).expect("read");
        let args = sexpr::list_args(&exprs[0]).expect("list");
        let head = sexpr::as_symbol(&args[0]).expect("head");
        EguiSugar
            .read_spec(&head, SugarArgs::new(&args[1..], text))
            .expect("read_spec")
    }

    #[test]
    fn inspect_round_trips() {
        let bare = EguiSugar.read_bare("inspect").expect("bare inspect");
        assert_eq!(bare.get("type").and_then(Datum::as_str), Some("Inspect"));
        assert_eq!(
            EguiSugar.write_spec("Inspect", &bare).as_deref(),
            Some("inspect")
        );
    }

    #[test]
    fn gui_round_trips() {
        let s = EguiSugar;

        // Bare default.
        let bare = s.read_bare("gui").expect("bare gui");
        assert_eq!(bare.get("type").and_then(Datum::as_str), Some("Gui"));
        assert_eq!(s.write_spec("Gui", &bare).as_deref(), Some("gui"));

        // Positional role.
        let view = read_spec("(gui view)").expect("gui view");
        assert_eq!(view.get("role").and_then(Datum::as_str), Some("view"));
        assert_eq!(s.write_spec("Gui", &view).as_deref(), Some("(gui view)"));

        // A display keyword keeps the role written even when it is the
        // default, so the positional slot stays unambiguous.
        let disp = read_spec("(gui body #:display compact)").expect("gui display");
        assert_eq!(disp.get("role").and_then(Datum::as_str), Some("body"));
        assert_eq!(disp.get("display").and_then(Datum::as_str), Some("compact"));
        assert_eq!(
            s.write_spec("Gui", &disp).as_deref(),
            Some("(gui body #:display compact)"),
        );

        // A display keyword without a positional role reads as the default
        // role and writes it back explicitly.
        let only_disp = read_spec("(gui #:display label)").expect("gui only display");
        assert_eq!(only_disp.get("role").and_then(Datum::as_str), Some("body"));
        assert_eq!(
            s.write_spec("Gui", &only_disp).as_deref(),
            Some("(gui body #:display label)"),
        );
    }

    #[test]
    fn comment_round_trips() {
        let s = EguiSugar;

        // Default size when none is given.
        let d = read_spec(r#"(comment "hi")"#).expect("comment");
        assert_eq!(d.get("text").and_then(Datum::as_str), Some("hi"));
        assert_eq!(
            s.write_spec("Comment", &d).as_deref(),
            Some(r#"(comment "hi" 100 40)"#),
        );

        // Explicit size round-trips.
        let sized = read_spec(r#"(comment "note" 220 80)"#).expect("sized");
        assert_eq!(
            s.write_spec("Comment", &sized).as_deref(),
            Some(r#"(comment "note" 220 80)"#),
        );
    }
}
