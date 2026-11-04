use crate::mini;
use crate::sugar::{KEY, MINI, PICK};
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

/// The hint of an empty key.
const KEY_HINT: &str = "freq";

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

/// Wrap each value of a pattern in a one-key control map.
///
/// The key is part of the node's identity, so editing it recompiles. An
/// empty key gives silence.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Pkey {
    key: String,
}

/// Project the maps of a schedule onto a key, for a control input.
///
/// The key is part of the node's identity, so editing it recompiles. An
/// empty key delivers nothing.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SchedPick {
    key: String,
}

impl Pkey {
    /// A node for the given key.
    pub fn new(key: impl Into<String>) -> Self {
        Pkey { key: key.into() }
    }

    /// The key.
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl SchedPick {
    /// A node for the given key.
    pub fn new(key: impl Into<String>) -> Self {
        SchedPick { key: key.into() }
    }

    /// The key.
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl NodeDef for Pkey {
    const TAG: &'static str = "Pkey";

    fn meta(&self) -> Meta {
        Meta {
            name: KEY.into(),
            description: "Wrap each value of a pattern in a one-key control map".into(),
            inputs: 1,
            outputs: 1,
            requires: vec!["gantz/pattern".into()],
            ..Meta::default()
        }
    }

    /// The combinator over the input pattern. An invalid key is a compile
    /// error of the node, as for invalid mini-notation.
    fn expr(&self, ctx: ExprArgs<'_>) -> Result<String, String> {
        let input = ctx.inputs.first().copied().flatten();
        Ok(match (key_lit(&self.key)?, input) {
            (Some(key), Some(p)) => format!("(pat/key {key} {p})"),
            _ => "pat/silence".to_string(),
        })
    }

    fn view(&self) -> View {
        View {
            inputs: vec![SocketDoc::new("pattern", "the values")],
            outputs: vec![SocketDoc::new("pattern", "one-key maps of the values")],
            body: Some(key_editor()),
            ..View::default()
        }
    }
}

impl NodeDef for SchedPick {
    const TAG: &'static str = "SchedPick";

    fn meta(&self) -> Meta {
        Meta {
            name: PICK.into(),
            description: "Project the maps of a schedule onto a key".into(),
            inputs: 1,
            outputs: 1,
            requires: vec!["gantz/sched".into()],
            ..Meta::default()
        }
    }

    /// The projection of the input schedule. An invalid key is a compile
    /// error of the node, as for invalid mini-notation.
    fn expr(&self, ctx: ExprArgs<'_>) -> Result<String, String> {
        let input = ctx.inputs.first().copied().flatten();
        Ok(match (key_lit(&self.key)?, input) {
            (Some(key), Some(schedule)) => format!("(sched/pick {schedule} {key})"),
            _ => "'()".to_string(),
        })
    }

    fn view(&self) -> View {
        View {
            inputs: vec![SocketDoc::new("schedule", "(seconds map) pairs")],
            outputs: vec![SocketDoc::new(
                "schedule",
                "(seconds value) pairs of the key",
            )],
            body: Some(key_editor()),
            ..View::default()
        }
    }
}

/// An editor of a node's `key` field. An edit commits once typing pauses or
/// the editor loses focus.
fn key_editor() -> gantz_ui::Element {
    gantz_ui::Element::TextInput(gantz_ui::TextInput {
        field: Some(gantz_ui::Field("key".into())),
        hint: Some(KEY_HINT.into()),
        mono: true,
        ..gantz_ui::TextInput::default()
    })
}

/// The Steel symbol literal of `key`, or `None` for an empty key. A key
/// holds letters, digits, `-` and `_`.
fn key_lit(key: &str) -> Result<Option<String>, String> {
    let valid = |c: char| c.is_alphanumeric() || c == '-' || c == '_';
    match key {
        "" => Ok(None),
        key if key.chars().all(valid) => Ok(Some(format!("'{key}"))),
        key => Err(format!("invalid key: {key:?}")),
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
        builtin(KEY, gantz_nodedef::node_data(&Pkey::default())),
        builtin(PICK, gantz_nodedef::node_data(&SchedPick::default())),
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

    fn key_ctx(
        input: Option<&str>,
        f: impl FnOnce(ExprArgs<'_>) -> Result<String, String>,
    ) -> Result<String, String> {
        f(ExprArgs {
            inputs: &[input],
            outputs: &[true],
            path: &[0],
            args: "%args",
        })
    }

    /// The keyed nodes wrap their input. An empty key or no input gives
    /// silence, and an invalid key is an error that names it.
    #[test]
    fn keyed_exprs() {
        let key = |k: &str, input| key_ctx(input, |ctx| Pkey::new(k).expr(ctx));
        let pick = |k: &str, input| key_ctx(input, |ctx| SchedPick::new(k).expr(ctx));
        assert_eq!(key("freq", Some("p")).unwrap(), "(pat/key 'freq p)");
        assert_eq!(pick("freq", Some("l")).unwrap(), "(sched/pick l 'freq)");
        assert_eq!(key("", Some("p")).unwrap(), "pat/silence");
        assert_eq!(key("freq", None).unwrap(), "pat/silence");
        assert_eq!(pick("", Some("l")).unwrap(), "'()");
        assert_eq!(pick("freq", None).unwrap(), "'()");
        assert!(key("a b", Some("p")).unwrap_err().contains("a b"));
        assert!(pick("a)", Some("l")).unwrap_err().contains("a)"));
    }
}
