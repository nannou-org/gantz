use crate::mini;
use crate::sugar::MINI;
use gantz_nodedef::{ExprArgs, Meta, NodeDef, SocketDoc, View};
use serde::{Deserialize, Serialize};

/// A mini-notation pattern, parsed at graph compile time.
///
/// The notation string is part of the node's identity, so editing it
/// recompiles. The expr embeds the parsed combinator source directly,
/// making the node pure and stateless.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Pmini {
    src: String,
}

/// The hint of an empty notation.
const HINT: &str = "bd(3,8) ~ [sn sn]";

impl Pmini {
    /// A node for the given notation source.
    pub fn new(src: impl Into<String>) -> Self {
        Pmini { src: src.into() }
    }

    /// The notation source.
    pub fn src(&self) -> &str {
        &self.src
    }
}

impl NodeDef for Pmini {
    const TAG: &'static str = "Pmini";

    fn meta(&self) -> Meta {
        Meta {
            name: MINI.into(),
            description: "Parse mini-notation into a pattern at compile time".into(),
            inputs: 1,
            outputs: 1,
            requires: vec!["gantz/pattern".into()],
            ..Meta::default()
        }
    }

    /// The parsed combinator source. Invalid notation is a compile error of
    /// the node, so the graph keeps the pattern that it compiled before.
    fn expr(&self, _ctx: ExprArgs<'_>) -> Result<String, String> {
        mini::steel_src(&self.src).ok_or_else(|| format!("invalid mini-notation: {:?}", self.src))
    }

    /// An editor of the notation. An edit commits once typing pauses or the
    /// editor loses focus, so a notation that is invalid in the middle of an
    /// edit does not raise its error at each keystroke.
    fn view(&self) -> View {
        let editor = gantz_ui::Element::TextInput(gantz_ui::TextInput {
            field: Some(gantz_ui::Field("src".into())),
            hint: Some(HINT.into()),
            mono: true,
            ..gantz_ui::TextInput::default()
        });
        View {
            inputs: vec![SocketDoc::new("bang", "triggers emission of the pattern")],
            outputs: vec![SocketDoc::new(
                "pattern",
                "the parsed pattern, silence when malformed",
            )],
            body: Some(editor),
            ..View::default()
        }
    }
}

/// The builtin node specs provided by this domain.
pub fn builtins() -> Vec<gantz_core::Builtin> {
    let builtin = |name, node: Result<gantz_ca::NodeData, String>| gantz_core::Builtin {
        name,
        node: node.unwrap_or_else(|e| panic!("builtin `{name}` failed to erase: {e}")),
    };
    vec![
        builtin(MINI, gantz_nodedef::node_data(&Pmini::default())),
        builtin(
            crate::plot::KEYWORD,
            gantz_nodedef::node_data(&crate::Pplot::default()),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expr(node: &Pmini) -> Result<String, String> {
        let ctx = ExprArgs {
            inputs: &[None],
            outputs: &[true],
            path: &[0],
            args: "%args",
        };
        node.expr(ctx)
    }

    /// The expr is exactly the parsed combinators, with no state and no
    /// wrapper. The default empty notation compiles to silence, and invalid
    /// notation is an error that names it.
    #[test]
    fn expr_emits_combinators() {
        assert_eq!(
            expr(&Pmini::new("bd sn")).unwrap(),
            "(pat/fastcat (list (pat/pure 'bd) (pat/pure 'sn)))",
        );
        assert_eq!(expr(&Pmini::default()).unwrap(), "pat/silence");
        assert!(expr(&Pmini::new("bd [")).unwrap_err().contains("bd ["));
    }
}
