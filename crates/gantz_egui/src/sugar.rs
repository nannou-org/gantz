//! `.gantz` keyword sugar for this crate's GUI node set.
//!
//! [`EguiSugar`] provides the keywords for the egui nodes. They are
//! `(bind <id>...)`, `(comment <text> [w h])`,
//! `(gui [<role>] [#:display <d>])` and bare `bind`,
//! `inspect` and `gui`. Compose it with [`gantz_format::CoreSugar`] and the
//! other crates' sugars via [`gantz_format::Sugars`].

use crate::node::{Bind, Comment, Gui, GuiDisplay, GuiRole, Inspect};
use gantz_format::sexpr::quote;
use gantz_format::{Datum, FormatError, Sugar, SugarArgs, node_datum};
use gantz_nodetag::NodeTag;

/// Keyword sugar for [`Bind`], [`Comment`], [`Gui`] and [`Inspect`].
#[derive(Clone, Copy, Debug, Default)]
pub struct EguiSugar;

/// Sugar keyword to node tag, for the egui builtins whose bare keyword lowers
/// to a default node. Explicit `read_spec` and `write_spec` arms handle
/// non-default `Comment` and `Gui` forms.
const KEYWORD_TAG: &[(&str, &str)] = &[
    ("bind", Bind::TAG),
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
            "bind" => bind_spec(args)?,
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
            "Bind" => Some(write_bind(node)),
            "Comment" => Some(write_comment(node)),
            // Must precede the bare-keyword fallback. A non-default `Gui`
            // written as bare `gui` would silently drop its role and display.
            "Gui" => Some(write_gui(node)),
            other => keyword_for_tag(other).map(str::to_string),
        }
    }

    fn keyword_for_tag(&self, tag: &str) -> Option<&str> {
        keyword_for_tag(tag)
    }
}

/// Read a `(bind <id>...)` form. Every positional is a node index. A bare
/// `(bind)` is the empty path.
fn bind_spec(args: SugarArgs<'_>) -> Result<Datum, FormatError> {
    let mut path = Vec::new();
    while let Some(id) = args.int_at(path.len())? {
        let id = u64::try_from(id)
            .map_err(|_| args.malformed_at(path.len(), "bind ids must be non-negative"))?;
        path.push(Datum::U64(id));
    }
    let fields = match path.is_empty() {
        true => vec![],
        false => vec![("path", Datum::Seq(path))],
    };
    Ok(node_datum("Bind", fields))
}

/// Write a `Bind` as a bare `bind` when it has no target, else
/// `(bind <id>...)`.
fn write_bind(node: &Datum) -> String {
    let ids: Vec<String> = node
        .get("path")
        .and_then(Datum::as_seq)
        .map(|seq| {
            seq.iter()
                .filter_map(Datum::as_i64)
                .map(|i| i.to_string())
                .collect()
        })
        .unwrap_or_default();
    match ids.is_empty() {
        true => "bind".to_string(),
        false => format!("(bind {})", ids.join(" ")),
    }
}

/// Read a `(comment <text> [w h])` form. The text is required. The `[w h]`
/// size is optional and defaults to `[100 40]`.
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

/// Read a `(gui [<role>] [#:display <d>])` form. The positional role symbol
/// is optional and defaults to `body`. The display keyword is optional and
/// defaults to `full`.
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

/// Write the canonical `gui` form. It is bare when all-default, and
/// `(gui <role>)` when only the role differs. When a display follows, the
/// role is always written, even `body`, so the positional slot stays
/// unambiguous.
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
    fn bind_round_trips() {
        let s = EguiSugar;

        // A bind with no target is bare, read as a keyword or as `(bind)`.
        let bare = s.read_bare("bind").expect("bare bind");
        assert_eq!(bare.get("type").and_then(Datum::as_str), Some("Bind"));
        assert!(bare.get("path").is_none());
        assert_eq!(s.write_spec("Bind", &bare).as_deref(), Some("bind"));
        let empty = read_spec("(bind)").expect("empty bind");
        assert_eq!(s.write_spec("Bind", &empty).as_deref(), Some("bind"));

        // A path of one or more ids round-trips.
        let one = read_spec("(bind 1)").expect("bind 1");
        assert_eq!(
            one.get("path").and_then(Datum::as_seq).map(|s| s.len()),
            Some(1)
        );
        assert_eq!(s.write_spec("Bind", &one).as_deref(), Some("(bind 1)"));
        let two = read_spec("(bind 1 2)").expect("bind 1 2");
        assert_eq!(s.write_spec("Bind", &two).as_deref(), Some("(bind 1 2)"));

        // A negative id is malformed.
        let exprs = sexpr::read("(bind -1)").expect("read");
        let args = sexpr::list_args(&exprs[0]).expect("list");
        assert!(
            s.read_spec("bind", SugarArgs::new(&args[1..], "(bind -1)"))
                .is_err()
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
