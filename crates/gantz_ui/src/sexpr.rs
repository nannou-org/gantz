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
}
