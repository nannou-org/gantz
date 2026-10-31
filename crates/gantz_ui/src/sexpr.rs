//! The s-expression text and the summary of a datum, for display and
//! diagnostics.
//!
//! The UI tree form is a [`Datum`]. Symbols are identifiers, sequences are
//! lists, and the other variants are values outside the form, which
//! diagnostics name by their summary.

use gantz_ca::Datum;

/// A short human readable description of a value for use in diagnostics,
/// phrased to follow "expected ..., found".
pub fn summary(expr: &Datum) -> String {
    match expr {
        Datum::Sym(s) => format!("the identifier `{s}`"),
        Datum::Bool(true) => "the boolean `#t`".to_string(),
        Datum::Bool(false) => "the boolean `#f`".to_string(),
        Datum::I64(i) => format!("the integer `{i}`"),
        Datum::U64(u) => format!("the integer `{u}`"),
        Datum::F64(f) => format!("the float `{f}`"),
        Datum::Ratio(r) => format!("the ratio `{}/{}`", r.numer(), r.denom()),
        Datum::Str(s) => format!("the string {:?}", truncated(s)),
        Datum::Seq(items) if items.is_empty() => "an empty list".to_string(),
        Datum::Seq(items) if items.len() == 1 => "a list of 1 item".to_string(),
        Datum::Seq(items) => format!("a list of {} items", items.len()),
        Datum::Null => "void".to_string(),
        Datum::Char(_) => "a character".to_string(),
        Datum::Bytes(_) => "a byte buffer".to_string(),
        Datum::Map(_) => match expr.as_opaque() {
            Some(what) => what.to_string(),
            None => "a map".to_string(),
        },
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
pub fn pretty(expr: &Datum, width: usize) -> String {
    let mut out = String::new();
    write_pretty(expr, 0, width, &mut out);
    out
}

/// The single-line scheme text of `expr`.
///
/// Symbols print verbatim, booleans as `#t` and `#f`, floats always with a
/// fraction, ratios as `n/d`, strings double-quoted with `"`, `\`, newline
/// and tab escaped, and other values outside the form as `#<summary>`.
pub fn flat(expr: &Datum) -> String {
    match expr {
        Datum::Sym(s) => s.clone(),
        Datum::Bool(true) => "#t".to_string(),
        Datum::Bool(false) => "#f".to_string(),
        Datum::I64(i) => i.to_string(),
        Datum::U64(u) => u.to_string(),
        Datum::F64(f) => format!("{f:?}"),
        Datum::Ratio(r) => format!("{}/{}", r.numer(), r.denom()),
        Datum::Str(s) => quoted(s),
        Datum::Seq(items) => {
            let items: Vec<String> = items.iter().map(flat).collect();
            format!("({})", items.join(" "))
        }
        Datum::Null | Datum::Char(_) | Datum::Bytes(_) | Datum::Map(_) => {
            format!("#<{}>", summary(expr))
        }
    }
}

/// Whether `expr` is an attribute block, a list headed by the `@` marker.
fn is_attrs(expr: &Datum) -> bool {
    matches!(expr, Datum::Seq(items) if items.first().and_then(Datum::as_sym) == Some("@"))
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

fn write_pretty(expr: &Datum, indent: usize, width: usize, out: &mut String) {
    let flat_text = flat(expr);
    let (head, rest) = match expr {
        Datum::Seq(items) if indent + flat_text.len() > width => match items.split_first() {
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
        assert_eq!(summary(&Datum::Sym("col".into())), "the identifier `col`");
        assert_eq!(summary(&Datum::Bool(true)), "the boolean `#t`");
        assert_eq!(summary(&Datum::Bool(false)), "the boolean `#f`");
        assert_eq!(summary(&Datum::I64(42)), "the integer `42`");
        assert_eq!(summary(&Datum::F64(1.5)), "the float `1.5`");
        assert_eq!(summary(&Datum::Str("hi".into())), "the string \"hi\"");
        assert_eq!(summary(&Datum::Char('x')), "a character");
        assert_eq!(summary(&Datum::ratio(1, 3).unwrap()), "the ratio `1/3`");
    }

    #[test]
    fn list_summaries_count_items() {
        assert_eq!(summary(&Datum::Seq(vec![])), "an empty list");
        assert_eq!(
            summary(&Datum::Seq(vec![Datum::I64(1)])),
            "a list of 1 item"
        );
        assert_eq!(
            summary(&Datum::Seq(vec![Datum::I64(1), Datum::I64(2)])),
            "a list of 2 items"
        );
    }

    #[test]
    fn long_strings_truncate() {
        let long = "x".repeat(100);
        let s = summary(&Datum::Str(long));
        assert_eq!(s, format!("the string \"{}...\"", "x".repeat(24)));
    }

    fn ident(s: &str) -> Datum {
        Datum::Sym(s.to_string())
    }

    fn list(items: Vec<Datum>) -> Datum {
        Datum::Seq(items)
    }

    fn attrs(items: Vec<Datum>) -> Datum {
        let mut all = vec![ident("@")];
        all.extend(items);
        list(all)
    }

    fn attr(name: &str, value: Datum) -> Datum {
        list(vec![ident(name), value])
    }

    fn bind(id: i64) -> Datum {
        attr("bind", list(vec![Datum::I64(id)]))
    }

    fn label(text: &str) -> Datum {
        attr("label", Datum::Str(text.to_string()))
    }

    /// The `demo.gui` body tree.
    fn adder() -> Datum {
        let dialer = |id, text| list(vec![ident("dialer"), attrs(vec![bind(id), label(text)])]);
        list(vec![
            ident("col"),
            list(vec![
                ident("frame"),
                attrs(vec![attr("title", Datum::Str("adder".into()))]),
                list(vec![ident("row"), dialer(1, "a"), dialer(2, "b")]),
                list(vec![ident("button"), attrs(vec![bind(0), label("add")])]),
                list(vec![
                    ident("row"),
                    list(vec![ident("label"), Datum::Str("a + b =".into())]),
                    list(vec![ident("value"), attrs(vec![bind(4)])]),
                ]),
            ]),
        ])
    }

    #[test]
    fn atoms_print_in_scheme_form() {
        assert_eq!(pretty(&ident("col"), 80), "col");
        assert_eq!(pretty(&Datum::Bool(true), 80), "#t");
        assert_eq!(pretty(&Datum::Bool(false), 80), "#f");
        assert_eq!(pretty(&Datum::I64(-3), 80), "-3");
        assert_eq!(pretty(&Datum::F64(1.0), 80), "1.0");
        assert_eq!(pretty(&Datum::F64(0.5), 80), "0.5");
        assert_eq!(
            pretty(&Datum::Str("say \"hi\"\n\\".into()), 80),
            r#""say \"hi\"\n\\""#
        );
        assert_eq!(pretty(&Datum::Null, 80), "#<void>");
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
            attrs(vec![attr("title", Datum::Str("adder".into()))]),
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
