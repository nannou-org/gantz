//! The key field of a node that reads or writes one key of a map.
//!
//! A keyed node stores its key as text in a `key` field. Its expr names the
//! key with the Steel symbol that [`lit`] gives, and its body edits the field
//! with the [`editor`].

use gantz_ui::{Element, Field, Label, Row, TextInput, Tone};

/// The Steel symbol literal of `key`, or `None` for an empty key. A key
/// holds letters, digits, `-` and `_`. Any other key is an error that names
/// it.
pub fn lit(key: &str) -> Result<Option<String>, String> {
    let valid = |c: char| c.is_alphanumeric() || c == '-' || c == '_';
    match key {
        "" => Ok(None),
        key if key.chars().all(valid) => Ok(Some(format!("'{key}"))),
        key => Err(format!("invalid key: {key:?}")),
    }
}

/// An editor of the `key` field, after a weak `label` that names what the
/// node does with the key, as a number shows its label. The field has no
/// box, so the node reads apart from code, and shows `hint` while it is
/// empty. An edit commits once typing pauses or the editor loses focus.
pub fn editor(label: &str, hint: &str) -> Element {
    let label = Element::Label(Label {
        text: label.into(),
        tone: Some(Tone::Weak),
        mono: true,
        ..Label::default()
    });
    let field = Element::TextInput(TextInput {
        field: Some(Field("key".into())),
        hint: Some(hint.into()),
        mono: true,
        frame: false,
        ..TextInput::default()
    });
    Element::Row(Row {
        children: vec![label, field],
        ..Row::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key becomes a quoted symbol. An empty key is no key, and a key with
    /// other characters is an error that names it.
    #[test]
    fn lit_quotes_valid_keys() {
        assert_eq!(lit("freq").unwrap().as_deref(), Some("'freq"));
        assert_eq!(lit("note-on_2").unwrap().as_deref(), Some("'note-on_2"));
        assert_eq!(lit("").unwrap(), None);
        assert!(lit("a b").unwrap_err().contains("a b"));
        assert!(lit("a)").unwrap_err().contains("a)"));
    }
}
