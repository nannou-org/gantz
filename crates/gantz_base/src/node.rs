//! The base nodes that are defined as data. See [`gantz_nodedef`].

use crate::sugar::{GET, LITERAL, SET};
use gantz_nodedef::{ExprArgs, Meta, NodeDef, SocketDoc, View, key};
use serde::{Deserialize, Serialize};

/// Read one key of a map.
///
/// The key is part of the node's identity, so editing it recompiles. A
/// missing key, a value that is not a map and an empty key all give the
/// empty list.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Get {
    key: String,
}

/// Set one key of a map.
///
/// The key is part of the node's identity, so editing it recompiles. A value
/// that is not a map counts as the empty map. An empty key, or no value,
/// leaves the map as it is.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Set {
    key: String,
}

/// A constant datum, such as a list of ratios or an exact rational, written
/// as Steel text.
///
/// The text is part of the node's identity, so editing it recompiles. The
/// expr is the quoted datum, so the node is pure and stateless. Empty text
/// gives the empty list.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Literal {
    src: String,
}

/// The hint of an empty key.
const KEY_HINT: &str = "width";

/// The hint of an empty literal.
const LITERAL_HINT: &str = "(1 9/8 5/4 3/2)";

impl Get {
    /// A node for the given key.
    pub fn new(key: impl Into<String>) -> Self {
        Get { key: key.into() }
    }
}

impl Set {
    /// A node for the given key.
    pub fn new(key: impl Into<String>) -> Self {
        Set { key: key.into() }
    }
}

impl Literal {
    /// A node for the given Steel text.
    pub fn new(src: impl Into<String>) -> Self {
        Literal { src: src.into() }
    }

    /// The Steel text.
    pub fn src(&self) -> &str {
        &self.src
    }
}

impl NodeDef for Get {
    const TAG: &'static str = "Get";

    fn meta(&self) -> Meta {
        Meta {
            name: GET.into(),
            description: "Read one key of a map".into(),
            inputs: 1,
            outputs: 1,
            ..Meta::default()
        }
    }

    /// The value under the key. An invalid key is a compile error of the
    /// node.
    fn expr(&self, ctx: ExprArgs<'_>) -> Result<String, String> {
        let input = ctx.inputs.first().copied().flatten();
        Ok(match (key::lit(&self.key)?, input) {
            (Some(key), Some(m)) => format!(
                "(let ((m {m})) \
                   (if (if (hash? m) (hash-contains? m {key}) #f) (hash-ref m {key}) '()))"
            ),
            _ => "'()".to_string(),
        })
    }

    fn view(&self) -> View {
        View {
            inputs: vec![SocketDoc::new("map", "a map")],
            outputs: vec![SocketDoc::new(
                "value",
                "the value under the key, or the empty list",
            )],
            body: Some(key::editor("get", KEY_HINT)),
            ..View::default()
        }
    }
}

impl NodeDef for Set {
    const TAG: &'static str = "Set";

    fn meta(&self) -> Meta {
        Meta {
            name: SET.into(),
            description: "Set one key of a map".into(),
            inputs: 2,
            outputs: 1,
            ..Meta::default()
        }
    }

    /// The map with the value under the key. An invalid key is a compile
    /// error of the node.
    fn expr(&self, ctx: ExprArgs<'_>) -> Result<String, String> {
        let input = |ix: usize| ctx.inputs.get(ix).copied().flatten();
        let map = match input(0) {
            Some(m) => format!("(let ((m {m})) (if (hash? m) m (hash)))"),
            None => "(hash)".to_string(),
        };
        Ok(match (key::lit(&self.key)?, input(1)) {
            (Some(key), Some(v)) => format!("(hash-insert {map} {key} {v})"),
            _ => map,
        })
    }

    fn view(&self) -> View {
        View {
            inputs: vec![
                SocketDoc::new("map", "a map, or nothing for the empty map"),
                SocketDoc::new("value", "the value to set"),
            ],
            outputs: vec![SocketDoc::new(
                "map",
                "the map with the value under the key",
            )],
            body: Some(key::editor("set", KEY_HINT)),
            ..View::default()
        }
    }
}

impl NodeDef for Literal {
    const TAG: &'static str = "Literal";

    fn meta(&self) -> Meta {
        Meta {
            name: LITERAL.into(),
            description: "A constant datum written as Steel text".into(),
            inputs: 1,
            outputs: 1,
            ..Meta::default()
        }
    }

    /// The quoted datum. Text that is not one datum is a compile error of
    /// the node, so the graph keeps the value that it compiled before. The
    /// close of the quote is on a line of its own, so a comment at the end
    /// of the text does not hide it.
    fn expr(&self, _ctx: ExprArgs<'_>) -> Result<String, String> {
        let invalid = || format!("invalid literal: {:?}", self.src);
        match gantz_format::sexpr::read(&self.src)
            .map_err(|_| invalid())?
            .len()
        {
            0 => Ok("'()".to_string()),
            1 => Ok(format!("(quote {}\n)", self.src.trim())),
            _ => Err(invalid()),
        }
    }

    /// An editor of the text. An edit commits once typing pauses or the
    /// editor loses focus, so text that is invalid in the middle of an edit
    /// does not raise its error at each keystroke.
    fn view(&self) -> View {
        let editor = gantz_ui::Element::TextInput(gantz_ui::TextInput {
            field: Some(gantz_ui::Field("src".into())),
            hint: Some(LITERAL_HINT.into()),
            mono: true,
            ..gantz_ui::TextInput::default()
        });
        View {
            inputs: vec![SocketDoc::new("bang", "emits the datum")],
            outputs: vec![SocketDoc::new("value", "the datum")],
            body: Some(editor),
            ..View::default()
        }
    }
}

/// The builtin node specs of the base node set.
pub fn builtins() -> Vec<gantz_core::Builtin> {
    let builtin = |name, node: Result<_, String>| gantz_core::Builtin {
        name,
        node: node.unwrap_or_else(|e| panic!("builtin `{name}` failed to erase: {e}")),
    };
    vec![
        builtin(GET, gantz_nodedef::node_data(&Get::default())),
        builtin(SET, gantz_nodedef::node_data(&Set::default())),
        builtin(LITERAL, gantz_nodedef::node_data(&Literal::default())),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expr(inputs: &[Option<&str>], node: &impl NodeDef) -> Result<String, String> {
        let ctx = ExprArgs {
            inputs,
            outputs: &[true],
            path: &[0],
            args: "%args",
        };
        node.expr(ctx)
    }

    /// `get` reads its key with the empty list for anything else. An empty
    /// key or no input gives the empty list, and an invalid key is an error
    /// that names it.
    #[test]
    fn get_expr() {
        assert_eq!(
            expr(&[Some("x")], &Get::new("width")).unwrap(),
            "(let ((m x)) (if (if (hash? m) (hash-contains? m 'width) #f) (hash-ref m 'width) '()))",
        );
        assert_eq!(expr(&[Some("x")], &Get::new("")).unwrap(), "'()");
        assert_eq!(expr(&[None], &Get::new("width")).unwrap(), "'()");
        assert!(
            expr(&[Some("x")], &Get::new("a b"))
                .unwrap_err()
                .contains("a b")
        );
    }

    /// `set` inserts into the map, or into the empty map. With no value or
    /// no key it gives the map as it is.
    #[test]
    fn set_expr() {
        let map = "(let ((m x)) (if (hash? m) m (hash)))";
        let node = Set::new("width");
        assert_eq!(
            expr(&[Some("x"), Some("v")], &node).unwrap(),
            format!("(hash-insert {map} 'width v)"),
        );
        assert_eq!(
            expr(&[None, Some("v")], &node).unwrap(),
            "(hash-insert (hash) 'width v)",
        );
        assert_eq!(expr(&[Some("x"), None], &node).unwrap(), map);
        assert_eq!(expr(&[Some("x"), Some("v")], &Set::new("")).unwrap(), map);
    }

    /// A literal quotes one datum. Empty text gives the empty list, and text
    /// that is not one datum is an error that names it.
    #[test]
    fn literal_expr() {
        let lit = |src: &str| expr(&[None], &Literal::new(src));
        assert_eq!(
            lit(" (1 9/8 5/4 3/2) ").unwrap(),
            "(quote (1 9/8 5/4 3/2)\n)"
        );
        assert_eq!(lit("1/3 ; a third").unwrap(), "(quote 1/3 ; a third\n)");
        assert_eq!(lit("bd").unwrap(), "(quote bd\n)");
        assert_eq!(lit("").unwrap(), "'()");
        assert!(lit("(1 2").unwrap_err().contains("(1 2"));
        assert!(lit("1) (2").unwrap_err().contains("1) (2"));
        assert!(lit("1 2").unwrap_err().contains("1 2"));
    }
}
