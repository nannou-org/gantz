use crate::mini;
use gantz_core::node::{ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_core::steel::SteelVal;
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

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

    fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
        let Some(pattern) = mini::steel_src(&self.src) else {
            return gantz_core::node::parse_expr("pat/silence");
        };
        let hash = {
            let mut h = std::hash::DefaultHasher::new();
            self.src.hash(&mut h);
            h.finish() as i64
        };
        gantz_core::node::parse_expr(&format!(
            "(if (if (pair? state) (equal? (car state) {hash}) #f) \
                 (cdr state) \
                 (begin (set! state (cons {hash} {pattern})) (cdr state)))"
        ))
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        true
    }

    fn register(&self, ctx: RegCtx<'_, '_>) {
        let (_, path, vm) = ctx.into_parts();
        gantz_core::node::state::init_value_if_absent(vm, path, || SteelVal::Void).unwrap();
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

    // The expr embeds the parsed combinators under a memo sentinel.
    #[test]
    fn expr_memoises_parsed_pattern() {
        let pm = Pm::new("bd sn");
        let src = expr_str(&pm);
        assert!(src.contains("(pat/fastcat (list (pat/pure (quote bd)) (pat/pure (quote sn))))"));
        assert!(src.contains("(set! state (cons"));
        // A different notation bakes a different sentinel.
        assert_ne!(src, expr_str(&Pm::new("bd sn cp")));
    }

    // Malformed notation compiles to silence.
    #[test]
    fn malformed_notation_is_silence() {
        assert_eq!(expr_str(&Pm::new("bd [")), "pat/silence");
    }
}
