use crate::mini;
use crate::sugar::{GATE, GET, KEY, LENSECS, MINI, PICK, SPLIT};
use gantz_nodedef::{Datum, ExprArgs, Meta, NodeDef, SocketDoc, View, key};
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

/// The hint of an empty key that holds seconds.
const SECS_HINT: &str = "dur";

/// The hint of an empty key that [`SchedSplit`] routes by.
const SPLIT_HINT: &str = "voice";

/// The most outputs of a [`SchedSplit`].
const SPLIT_MAX: usize = 32;

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

/// Read one key of each map of a pattern, the inverse of [`Pkey`].
///
/// The key is part of the node's identity, so editing it recompiles. An
/// empty key gives silence.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Pget {
    key: String,
}

/// Add the seconds of each event's whole to its map under a key, at a
/// steady or patterned cps.
///
/// The key is part of the node's identity, so editing it recompiles. An
/// empty key leaves the pattern as it is.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Plensecs {
    key: String,
}

/// Open a gate at each pair of a schedule, and close it after the seconds
/// under a key of the pair's map.
///
/// The key is part of the node's identity, so editing it recompiles. An
/// empty key gives no gates.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SchedGate {
    key: String,
}

/// Route each pair of a schedule to one of `n` outputs by the integer under
/// a key of its map, modulo `n`.
///
/// The key and the count are part of the node's identity, so editing them
/// recompiles. An empty key routes nothing.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SchedSplit {
    key: String,
    n: usize,
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

impl Pget {
    /// A node for the given key.
    pub fn new(key: impl Into<String>) -> Self {
        Pget { key: key.into() }
    }
}

impl Plensecs {
    /// A node for the given key.
    pub fn new(key: impl Into<String>) -> Self {
        Plensecs { key: key.into() }
    }
}

impl SchedGate {
    /// A node for the given key.
    pub fn new(key: impl Into<String>) -> Self {
        SchedGate { key: key.into() }
    }
}

impl SchedSplit {
    /// A node for the given key and count of outputs.
    pub fn new(key: impl Into<String>, n: usize) -> Self {
        SchedSplit { key: key.into(), n }
    }

    /// The count of outputs, from 1 to [`SPLIT_MAX`].
    fn outputs(&self) -> usize {
        self.n.clamp(1, SPLIT_MAX)
    }
}

impl Default for SchedSplit {
    fn default() -> Self {
        SchedSplit::new("", 2)
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
        Ok(match (key::lit(&self.key)?, input) {
            (Some(key), Some(p)) => format!("(pat/key {key} {p})"),
            _ => "pat/silence".to_string(),
        })
    }

    fn view(&self) -> View {
        View {
            inputs: vec![SocketDoc::new("pattern", "the values")],
            outputs: vec![SocketDoc::new("pattern", "one-key maps of the values")],
            body: Some(key::editor("key", KEY_HINT)),
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
        Ok(match (key::lit(&self.key)?, input) {
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
            body: Some(key::editor("pick", KEY_HINT)),
            ..View::default()
        }
    }
}

impl NodeDef for Pget {
    const TAG: &'static str = "Pget";

    fn meta(&self) -> Meta {
        Meta {
            name: GET.into(),
            description: "Read one key of each map of a pattern".into(),
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
        Ok(match (key::lit(&self.key)?, input) {
            (Some(key), Some(p)) => format!("(pat/get {key} {p})"),
            _ => "pat/silence".to_string(),
        })
    }

    fn view(&self) -> View {
        View {
            inputs: vec![SocketDoc::new("pattern", "maps")],
            outputs: vec![SocketDoc::new("pattern", "the values of the key")],
            body: Some(key::editor("get", KEY_HINT)),
            ..View::default()
        }
    }
}

impl NodeDef for Plensecs {
    const TAG: &'static str = "Plensecs";

    fn meta(&self) -> Meta {
        Meta {
            name: LENSECS.into(),
            description: "Add the seconds of each event to its map".into(),
            inputs: 3,
            outputs: 1,
            requires: vec!["gantz/pattern".into()],
            ..Meta::default()
        }
    }

    /// The combinator over the input pattern. An empty key leaves the pattern
    /// as it is. An invalid key is a compile error of the node, as for invalid
    /// mini-notation.
    fn expr(&self, ctx: ExprArgs<'_>) -> Result<String, String> {
        let input = |ix: usize| ctx.inputs.get(ix).copied().flatten();
        Ok(match (key::lit(&self.key)?, input(0)) {
            (_, None) => "pat/silence".to_string(),
            (None, Some(p)) => p.to_string(),
            (Some(key), Some(p)) => {
                let cps = input(1).unwrap_or("'()");
                let held = input(2).unwrap_or("'()");
                format!("(pat/len-secs {key} {p} {cps} {held})")
            }
        })
    }

    fn view(&self) -> View {
        View {
            inputs: vec![
                SocketDoc::new("pattern", "the events"),
                SocketDoc::new("cps", "a number or a pattern of numbers"),
                SocketDoc::new("number", "cps that a gap in the cps holds"),
            ],
            outputs: vec![SocketDoc::new(
                "pattern",
                "maps with the seconds of each event under the key",
            )],
            body: Some(key::editor("lensecs", SECS_HINT)),
            ..View::default()
        }
    }
}

impl NodeDef for SchedGate {
    const TAG: &'static str = "SchedGate";

    fn meta(&self) -> Meta {
        Meta {
            name: GATE.into(),
            description: "Open a gate at each pair of a schedule for the seconds under a key"
                .into(),
            inputs: 1,
            outputs: 1,
            requires: vec!["gantz/sched".into()],
            ..Meta::default()
        }
    }

    /// The gates of the input schedule. An invalid key is a compile error of
    /// the node, as for invalid mini-notation.
    fn expr(&self, ctx: ExprArgs<'_>) -> Result<String, String> {
        let input = ctx.inputs.first().copied().flatten();
        Ok(match (key::lit(&self.key)?, input) {
            (Some(key), Some(schedule)) => format!("(sched/gate {schedule} {key})"),
            _ => "'()".to_string(),
        })
    }

    fn view(&self) -> View {
        View {
            inputs: vec![SocketDoc::new("schedule", "(seconds map) pairs")],
            outputs: vec![SocketDoc::new(
                "schedule",
                "(seconds gate) pairs, 1.0 to open and 0.0 to close",
            )],
            body: Some(key::editor("gate", SECS_HINT)),
            ..View::default()
        }
    }
}

impl NodeDef for SchedSplit {
    const TAG: &'static str = "SchedSplit";

    fn meta(&self) -> Meta {
        Meta {
            name: SPLIT.into(),
            description: "Route each pair of a schedule by the integer under a key".into(),
            inputs: 1,
            outputs: self.outputs(),
            requires: vec!["gantz/sched".into()],
            ..Meta::default()
        }
    }

    /// The schedule of each output. An invalid key is a compile error of the
    /// node, as for invalid mini-notation.
    fn expr(&self, ctx: ExprArgs<'_>) -> Result<String, String> {
        let input = ctx.inputs.first().copied().flatten();
        let n = self.outputs();
        let split = match (key::lit(&self.key)?, input) {
            (Some(key), Some(schedule)) => format!("(sched/split {schedule} {key} {n})"),
            _ => format!("(list {})", vec!["'()"; n].join(" ")),
        };
        // A node with one output gives its value, not a list of one.
        Ok(match n {
            1 => format!("(car {split})"),
            _ => split,
        })
    }

    fn view(&self) -> View {
        let n = self.outputs();
        let count = gantz_ui::Element::Dialer(gantz_ui::Dialer {
            on: Some(gantz_ui::Edit("n".into())),
            value: Some(n as f64),
            min: Some(1.0),
            max: Some(SPLIT_MAX as f64),
            step: Some(1.0),
            precision: Some(0),
            tip: Some("the number of outputs".into()),
            ..gantz_ui::Dialer::default()
        });
        let body = gantz_ui::Element::Row(gantz_ui::Row {
            children: vec![key::editor("split", SPLIT_HINT), count],
            ..gantz_ui::Row::default()
        });
        View {
            inputs: vec![SocketDoc::new("schedule", "(seconds map) pairs")],
            outputs: (0..n)
                .map(|i| {
                    let doc = format!("the pairs whose key is {i} modulo {n}");
                    SocketDoc::new("schedule", &doc)
                })
                .collect(),
            body: Some(body),
            ..View::default()
        }
    }

    fn edit(&self, edit: &str, value: &Datum) -> Option<Self> {
        let n = match edit {
            "n" => value.as_f64()?.round().clamp(1.0, SPLIT_MAX as f64) as usize,
            _ => return None,
        };
        (n != self.n).then(|| SchedSplit {
            key: self.key.clone(),
            n,
        })
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
        builtin(GET, gantz_nodedef::node_data(&Pget::default())),
        builtin(LENSECS, gantz_nodedef::node_data(&Plensecs::default())),
        builtin(GATE, gantz_nodedef::node_data(&SchedGate::default())),
        builtin(SPLIT, gantz_nodedef::node_data(&SchedSplit::default())),
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
        let get = |k: &str, input| key_ctx(input, |ctx| Pget::new(k).expr(ctx));
        let gate = |k: &str, input| key_ctx(input, |ctx| SchedGate::new(k).expr(ctx));
        assert_eq!(get("cps", Some("p")).unwrap(), "(pat/get 'cps p)");
        assert_eq!(get("", Some("p")).unwrap(), "pat/silence");
        assert_eq!(gate("dur", Some("l")).unwrap(), "(sched/gate l 'dur)");
        assert_eq!(gate("dur", None).unwrap(), "'()");
        assert!(gate("a b", Some("l")).unwrap_err().contains("a b"));
    }

    fn exprs(inputs: &[Option<&str>], outputs: usize, node: &impl NodeDef) -> String {
        let outputs = vec![true; outputs];
        let ctx = ExprArgs {
            inputs,
            outputs: &outputs,
            path: &[0],
            args: "%args",
        };
        node.expr(ctx).unwrap()
    }

    /// The seconds of each event go under the key. An empty key leaves the
    /// pattern as it is, and a missing cps passes the empty list, which the
    /// combinator reads as no cps.
    #[test]
    fn lensecs_expr() {
        let node = Plensecs::new("dur");
        let all = [Some("p"), Some("c"), Some("h")];
        assert_eq!(exprs(&all, 1, &node), "(pat/len-secs 'dur p c h)");
        let no_cps = [Some("p"), None, Some("h")];
        assert_eq!(exprs(&no_cps, 1, &node), "(pat/len-secs 'dur p '() h)");
        assert_eq!(exprs(&all, 1, &Plensecs::new("")), "p");
        assert_eq!(exprs(&[None, None, None], 1, &node), "pat/silence");
    }

    /// Each output takes one schedule of the split. One output gives its
    /// schedule, not a list of one, and an edit of the count clamps it.
    #[test]
    fn split_expr_and_edit() {
        let node = SchedSplit::new("v", 3);
        assert_eq!(node.meta().outputs, 3);
        assert_eq!(exprs(&[Some("l")], 3, &node), "(sched/split l 'v 3)");
        assert_eq!(exprs(&[None], 3, &node), "(list '() '() '())");
        let one = SchedSplit::new("v", 1);
        assert_eq!(exprs(&[Some("l")], 1, &one), "(car (sched/split l 'v 1))");
        assert_eq!(node.view().outputs.len(), 3);
        let edit = |n: f64| node.edit("n", &Datum::F64(n)).map(|s| s.n);
        assert_eq!(edit(4.4), Some(4));
        assert_eq!(edit(0.0), Some(1));
        assert_eq!(edit(99.0), Some(SPLIT_MAX));
        assert_eq!(edit(3.0), None);
    }
}
