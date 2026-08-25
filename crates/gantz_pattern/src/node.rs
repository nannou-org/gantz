use crate::mini;
use gantz_core::node::{ExprCtx, ExprResult, MetaCtx};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};

/// A mini-notation pattern, parsed at graph compile time.
///
/// The notation string is part of the node's identity, so editing it
/// recompiles. The expr embeds the parsed combinator source, memoised in
/// node state under a source-hash sentinel so the pattern is constructed
/// once per edit and the steady-state cost per eval is a state read.
/// Malformed notation compiles to silence.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize, NodeTag)]
pub struct Pm {
    src: String,
}

impl Pm {
    /// A node for the given notation source.
    pub fn new(src: impl Into<String>) -> Self {
        Pm { src: src.into() }
    }

    /// The notation source.
    pub fn src(&self) -> &str {
        &self.src
    }

    /// Replace the notation source.
    pub fn set_src(&mut self, src: impl Into<String>) {
        self.src = src.into();
    }

    /// Whether the current notation parses.
    pub fn parses(&self) -> bool {
        mini::steel_src(&self.src).is_some()
    }
}

impl gantz_core::Node for Pm {
    /// A single bang input triggering emission of the pattern.
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    /// Emits the parsed combinator source directly. Invalid notation is a
    /// compile error attributed to the node, like an invalid expr - the
    /// node's editor refuses to commit unparseable buffers, so this only
    /// fires for file-authored or deserialized sources.
    fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
        match mini::steel_src(&self.src) {
            Some(pattern) => gantz_core::node::parse_expr(&pattern),
            None => Err(gantz_core::node::ExprError::custom(format!(
                "invalid mini-notation: {:?}",
                self.src,
            ))),
        }
    }

    fn required_modules(&self, _ctx: MetaCtx) -> Vec<String> {
        vec!["gantz/pattern".to_string()]
    }
}

/// The builtin node specs provided by this domain.
pub fn builtins() -> Vec<gantz_core::Builtin> {
    vec![gantz_core::Builtin::new("pm", &Pm::default())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_core::Node;

    fn expr_str(pm: &Pm) -> String {
        let outputs = gantz_core::node::Conns::try_from([true]).unwrap();
        let inputs = [None];
        let ctx = gantz_core::node::ExprCtx::new(&|_| None, &[], &inputs, &outputs);
        pm.expr(ctx).unwrap().to_pretty(200)
    }

    // The expr is exactly the parsed combinators - no state, no wrapper.
    #[test]
    fn expr_emits_combinators() {
        assert_eq!(
            expr_str(&Pm::new("bd sn")),
            "(pat/fastcat (list (pat/pure (quote bd)) (pat/pure (quote sn))))",
        );
        // The default (empty) notation compiles to silence.
        assert_eq!(expr_str(&Pm::default()), "pat/silence");
    }

    // Malformed notation is a compile error naming the source, mirroring
    // an invalid expr node.
    #[test]
    fn malformed_notation_errors() {
        let pm = Pm::new("bd [");
        let outputs = gantz_core::node::Conns::try_from([true]).unwrap();
        let inputs = [None];
        let ctx = gantz_core::node::ExprCtx::new(&|_| None, &[], &inputs, &outputs);
        let err = pm.expr(ctx).unwrap_err();
        assert!(
            format!("{err}").contains("bd ["),
            "err names the source: {err}"
        );
    }
}
