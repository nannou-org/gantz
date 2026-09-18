//! The abstract data model of the UI tree form.
//!
//! The vocabulary is specified over this model rather than over any one
//! runtime's value type. The model has identifier atoms, booleans, integers,
//! floats, strings and lists. Each runtime codec lowers its own value type
//! into [`SExpr`] totally, so the decoder is written once and every backend
//! shares it. Values outside the model, such as closures, maps and byte
//! buffers, lower to [`SExpr::Other`] with a short description so
//! diagnostics can name what was found.

/// A runtime neutral s-expression value, the seam every codec lowers into.
#[derive(Clone, Debug, PartialEq)]
pub enum SExpr {
    /// An identifier atom. A symbol in Steel, a string in `Datum`.
    Ident(String),
    /// A boolean.
    Bool(bool),
    /// An integer.
    Int(i64),
    /// A float. Codecs only produce finite floats and lower non-finite
    /// numbers to [`SExpr::Other`].
    Float(f64),
    /// A string.
    Str(String),
    /// A list of values.
    List(Vec<SExpr>),
    /// A runtime value outside the abstract model, carrying a short human
    /// readable description for diagnostics.
    Other(String),
}

/// A short human readable description of a value for use in diagnostics,
/// phrased to follow "expected ..., found".
pub fn summary(expr: &SExpr) -> String {
    match expr {
        SExpr::Ident(s) => format!("the identifier `{s}`"),
        SExpr::Bool(true) => "the boolean `#t`".to_string(),
        SExpr::Bool(false) => "the boolean `#f`".to_string(),
        SExpr::Int(i) => format!("the integer `{i}`"),
        SExpr::Float(f) => format!("the float `{f}`"),
        SExpr::Str(s) => format!("the string {:?}", truncated(s)),
        SExpr::List(items) if items.is_empty() => "an empty list".to_string(),
        SExpr::List(items) if items.len() == 1 => "a list of 1 item".to_string(),
        SExpr::List(items) => format!("a list of {} items", items.len()),
        SExpr::Other(s) => s.clone(),
    }
}

/// Cap a string for inclusion in a diagnostic message.
fn truncated(s: &str) -> String {
    const MAX: usize = 24;
    if s.chars().count() <= MAX {
        s.to_string()
    } else {
        let head: String = s.chars().take(MAX).collect();
        format!("{head}...")
    }
}

/// Print `expr` as indented scheme text.
///
/// A list prints on one line when it fits within `width` from its indent.
/// Otherwise its head opens the line, an attribute block that fits stays on
/// that line, and every other item prints on its own line two columns in.
/// `width` is a soft target. An atom or a list that cannot break any
/// further still prints whole.
pub fn pretty(expr: &SExpr, width: usize) -> String {
    let mut out = String::new();
    write_pretty(expr, 0, width, &mut out);
    out
}

/// The single-line scheme text of `expr`.
///
/// Identifiers print verbatim, booleans as `#t` and `#f`, floats always
/// with a fraction, strings double-quoted with `"`, `\`, newline and tab
/// escaped, and [`SExpr::Other`] as `#<description>`.
pub fn flat(expr: &SExpr) -> String {
    match expr {
        SExpr::Ident(s) => s.clone(),
        SExpr::Bool(true) => "#t".to_string(),
        SExpr::Bool(false) => "#f".to_string(),
        SExpr::Int(i) => i.to_string(),
        SExpr::Float(f) => format!("{f:?}"),
        SExpr::Str(s) => quoted(s),
        SExpr::List(items) => {
            let items: Vec<String> = items.iter().map(flat).collect();
            format!("({})", items.join(" "))
        }
        SExpr::Other(s) => format!("#<{s}>"),
    }
}

/// Whether `expr` is an attribute block, a list headed by the `@` marker.
fn is_attrs(expr: &SExpr) -> bool {
    matches!(expr, SExpr::List(items) if items.first() == Some(&SExpr::Ident("@".to_string())))
}

/// A double-quoted string literal.
fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn write_pretty(expr: &SExpr, indent: usize, width: usize, out: &mut String) {
    let flat_text = flat(expr);
    let (head, rest) = match expr {
        SExpr::List(items) if indent + flat_text.len() > width => match items.split_first() {
            Some(split) => split,
            None => {
                out.push_str(&flat_text);
                return;
            }
        },
        _ => {
            out.push_str(&flat_text);
            return;
        }
    };
    let head_text = flat(head);
    out.push('(');
    out.push_str(&head_text);
    let mut rest = rest;
    if let Some((attrs, after)) = rest.split_first() {
        let attrs_text = flat(attrs);
        if is_attrs(attrs) && indent + 2 + head_text.len() + attrs_text.len() <= width {
            out.push(' ');
            out.push_str(&attrs_text);
            rest = after;
        }
    }
    let pad = " ".repeat(indent + 2);
    for item in rest {
        out.push('\n');
        out.push_str(&pad);
        write_pretty(item, indent + 2, width, out);
    }
    out.push(')');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries_name_each_kind() {
        assert_eq!(summary(&SExpr::Ident("col".into())), "the identifier `col`");
        assert_eq!(summary(&SExpr::Bool(true)), "the boolean `#t`");
        assert_eq!(summary(&SExpr::Bool(false)), "the boolean `#f`");
        assert_eq!(summary(&SExpr::Int(42)), "the integer `42`");
        assert_eq!(summary(&SExpr::Float(1.5)), "the float `1.5`");
        assert_eq!(summary(&SExpr::Str("hi".into())), "the string \"hi\"");
        assert_eq!(summary(&SExpr::Other("a closure".into())), "a closure");
    }

    #[test]
    fn list_summaries_count_items() {
        assert_eq!(summary(&SExpr::List(vec![])), "an empty list");
        assert_eq!(
            summary(&SExpr::List(vec![SExpr::Int(1)])),
            "a list of 1 item"
        );
        assert_eq!(
            summary(&SExpr::List(vec![SExpr::Int(1), SExpr::Int(2)])),
            "a list of 2 items"
        );
    }

    #[test]
    fn long_strings_truncate() {
        let long = "x".repeat(100);
        let s = summary(&SExpr::Str(long));
        assert_eq!(s, format!("the string \"{}...\"", "x".repeat(24)));
    }

    fn ident(s: &str) -> SExpr {
        SExpr::Ident(s.to_string())
    }

    fn list(items: Vec<SExpr>) -> SExpr {
        SExpr::List(items)
    }

    fn attrs(items: Vec<SExpr>) -> SExpr {
        let mut all = vec![ident("@")];
        all.extend(items);
        list(all)
    }

    fn attr(name: &str, value: SExpr) -> SExpr {
        list(vec![ident(name), value])
    }

    fn bind(id: i64) -> SExpr {
        attr("bind", list(vec![SExpr::Int(id)]))
    }

    fn label(text: &str) -> SExpr {
        attr("label", SExpr::Str(text.to_string()))
    }

    /// The `demo-gui` body tree.
    fn adder() -> SExpr {
        let dialer = |id, text| list(vec![ident("dialer"), attrs(vec![bind(id), label(text)])]);
        list(vec![
            ident("col"),
            list(vec![
                ident("frame"),
                attrs(vec![attr("title", SExpr::Str("adder".into()))]),
                list(vec![ident("row"), dialer(1, "a"), dialer(2, "b")]),
                list(vec![ident("button"), attrs(vec![bind(0), label("add")])]),
                list(vec![
                    ident("row"),
                    list(vec![ident("label"), SExpr::Str("a + b =".into())]),
                    list(vec![ident("value"), attrs(vec![bind(4)])]),
                ]),
            ]),
        ])
    }

    #[test]
    fn atoms_print_in_scheme_form() {
        assert_eq!(pretty(&ident("col"), 80), "col");
        assert_eq!(pretty(&SExpr::Bool(true), 80), "#t");
        assert_eq!(pretty(&SExpr::Bool(false), 80), "#f");
        assert_eq!(pretty(&SExpr::Int(-3), 80), "-3");
        assert_eq!(pretty(&SExpr::Float(1.0), 80), "1.0");
        assert_eq!(pretty(&SExpr::Float(0.5), 80), "0.5");
        assert_eq!(
            pretty(&SExpr::Str("say \"hi\"\n\\".into()), 80),
            r#""say \"hi\"\n\\""#
        );
        assert_eq!(pretty(&SExpr::Other("void".into()), 80), "#<void>");
    }

    #[test]
    fn lists_that_fit_stay_flat() {
        assert_eq!(pretty(&list(vec![]), 80), "()");
        let tree = list(vec![ident("col"), list(vec![ident("sep")])]);
        assert_eq!(pretty(&tree, 80), "(col (sep))");
        // A width too narrow for even the head keeps an unbreakable list whole.
        assert_eq!(pretty(&list(vec![ident("sep")]), 2), "(sep)");
    }

    #[test]
    fn long_lists_break_with_attrs_on_the_head_line() {
        let expected = "\
(col
  (frame (@ (title \"adder\"))
    (row
      (dialer (@ (bind (1)) (label \"a\")))
      (dialer (@ (bind (2)) (label \"b\"))))
    (button (@ (bind (0)) (label \"add\")))
    (row (label \"a + b =\") (value (@ (bind (4)))))))";
        assert_eq!(pretty(&adder(), 60), expected);
    }

    #[test]
    fn attrs_that_do_not_fit_break_too() {
        let tree = list(vec![
            ident("frame"),
            attrs(vec![attr("title", SExpr::Str("adder".into()))]),
            list(vec![ident("sep")]),
        ]);
        let expected = "\
(frame
  (@
    (title \"adder\"))
  (sep))";
        assert_eq!(pretty(&tree, 20), expected);
    }
}
