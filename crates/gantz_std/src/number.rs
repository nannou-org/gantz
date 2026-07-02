use gantz_ca::CaHash;
use gantz_core::node::{EvalConf, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_core::steel::SteelVal;
use gantz_format::NodeTag;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

/// A number stored in state. Can be updated via the first input.
///
/// Optional configuration:
/// - `min`/`max` clamp every value (including input-socket values).
/// - `precision` controls how many decimals the dialer shows/edits (display
///   only).
/// - `push_eval_on_edit` toggles whether editing the dialer fires downstream.
///
/// Each field is folded into the content address only when non-default (see
/// [`CaHash`]): a plain `number` keeps the original address, while any
/// configured field becomes part of the node's identity so it persists and is
/// undoable under the commit-on-change model.
#[derive(Clone, Debug, Serialize, Deserialize, NodeTag)]
pub struct Number {
    #[serde(default)]
    min: Option<f64>,
    #[serde(default)]
    max: Option<f64>,
    #[serde(default)]
    precision: Option<u8>,
    #[serde(default = "default_push_eval")]
    push_eval_on_edit: bool,
}

impl Number {
    /// The lower bound the value is clamped to, if any.
    pub fn min(&self) -> Option<f64> {
        self.min
    }

    /// The upper bound the value is clamped to, if any.
    pub fn max(&self) -> Option<f64> {
        self.max
    }

    /// The number of decimal places the dialer shows/edits, if configured.
    pub fn precision(&self) -> Option<u8> {
        self.precision
    }

    /// Whether editing the dialer fires a push-eval downstream.
    pub fn push_eval_on_edit(&self) -> bool {
        self.push_eval_on_edit
    }

    /// Set the lower bound (content-address affecting).
    pub fn set_min(&mut self, min: Option<f64>) {
        self.min = min;
    }

    /// Set the upper bound (content-address affecting).
    pub fn set_max(&mut self, max: Option<f64>) {
        self.max = max;
    }

    /// Set the dialer display precision (UI-only).
    pub fn set_precision(&mut self, precision: Option<u8>) {
        self.precision = precision;
    }

    /// Set whether editing the dialer fires downstream (UI-only).
    pub fn set_push_eval_on_edit(&mut self, push_eval_on_edit: bool) {
        self.push_eval_on_edit = push_eval_on_edit;
    }

    /// Clamp `v` to the configured `min`/`max` bounds.
    pub fn clamp(&self, v: f64) -> f64 {
        let v = self.min.map_or(v, |lo| v.max(lo));
        self.max.map_or(v, |hi| v.min(hi))
    }
}

impl Default for Number {
    fn default() -> Self {
        Number {
            min: None,
            max: None,
            precision: None,
            push_eval_on_edit: true,
        }
    }
}

impl PartialEq for Number {
    fn eq(&self, other: &Self) -> bool {
        self.min.map(f64::to_bits) == other.min.map(f64::to_bits)
            && self.max.map(f64::to_bits) == other.max.map(f64::to_bits)
            && self.precision == other.precision
            && self.push_eval_on_edit == other.push_eval_on_edit
    }
}

impl Eq for Number {}

impl Hash for Number {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Fully qualified to disambiguate from `gantz_ca::CaHash::hash`.
        Hash::hash(&self.min.map(f64::to_bits), state);
        Hash::hash(&self.max.map(f64::to_bits), state);
        Hash::hash(&self.precision, state);
        Hash::hash(&self.push_eval_on_edit, state);
    }
}

impl CaHash for Number {
    fn hash(&self, hasher: &mut gantz_ca::Hasher) {
        hasher.update("gantz.number".as_bytes());
        // Each config field is folded in only when non-default, so a plain
        // `number` hashes to just the tag - byte-for-byte the old unit-struct
        // address, keeping existing `number` nodes stable. Configuring any field
        // gives the node a new address, which is how the commit-on-change model
        // persists it (the working graph is only saved when it commits).
        if let Some(min) = self.min {
            hasher.update(b"min");
            CaHash::hash(&min.to_bits(), hasher);
        }
        if let Some(max) = self.max {
            hasher.update(b"max");
            CaHash::hash(&max.to_bits(), hasher);
        }
        if let Some(precision) = self.precision {
            hasher.update(b"precision");
            CaHash::hash(&precision, hasher);
        }
        if !self.push_eval_on_edit {
            hasher.update(b"no-push-eval");
        }
    }
}

impl gantz_core::Node for Number {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn push_eval(&self, _ctx: MetaCtx) -> Vec<EvalConf> {
        vec![EvalConf::All]
    }

    fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult {
        let expr = match ctx.inputs().get(0) {
            // If an input value was provided, clamp it, use it to update state
            // and forward that value.
            Some(Some(val)) => {
                let stored = clamp_steel(val, self.min, self.max);
                format!("(begin (if (number? {val}) (set! state {stored}) void) state)")
            }
            // If no input value was provided, forward the value in state.
            _ => "(begin state)".to_string(),
        };
        gantz_core::node::parse_expr(&expr)
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        true
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        let path = ctx.path();
        let init = self.clamp(0.0);
        gantz_core::node::state::init_value_if_absent(ctx.vm(), path, || SteelVal::NumV(init))
            .unwrap()
    }
}

fn default_push_eval() -> bool {
    true
}

/// Build a Steel expression that clamps `val` to the given bounds.
///
/// `min`/`max` are unavailable in `Engine::new_base`, so this emits primitive
/// `if`s. `val` is bound once with `let` to avoid evaluating it twice.
fn clamp_steel(val: &str, min: Option<f64>, max: Option<f64>) -> String {
    match (min, max) {
        (None, None) => val.to_string(),
        (Some(lo), None) => format!("(let ((v {val})) (if (< v {lo:?}) {lo:?} v))"),
        (None, Some(hi)) => format!("(let ((v {val})) (if (> v {hi:?}) {hi:?} v))"),
        (Some(lo), Some(hi)) => {
            format!("(let ((v {val})) (if (< v {lo:?}) {lo:?} (if (> v {hi:?}) {hi:?} v)))")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_bounds() {
        let n = Number {
            min: Some(0.0),
            max: Some(10.0),
            precision: None,
            push_eval_on_edit: true,
        };
        assert_eq!(n.clamp(-5.0), 0.0);
        assert_eq!(n.clamp(5.0), 5.0);
        assert_eq!(n.clamp(15.0), 10.0);

        let lo = Number {
            min: Some(3.0),
            ..Number::default()
        };
        assert_eq!(lo.clamp(1.0), 3.0);
        assert_eq!(lo.clamp(100.0), 100.0);
    }

    #[test]
    fn clamp_steel_forms() {
        assert_eq!(clamp_steel("x", None, None), "x");
        assert_eq!(
            clamp_steel("x", Some(0.0), None),
            "(let ((v x)) (if (< v 0.0) 0.0 v))",
        );
        assert_eq!(
            clamp_steel("x", None, Some(10.0)),
            "(let ((v x)) (if (> v 10.0) 10.0 v))",
        );
        assert_eq!(
            clamp_steel("x", Some(0.0), Some(10.0)),
            "(let ((v x)) (if (< v 0.0) 0.0 (if (> v 10.0) 10.0 v)))",
        );
    }
}
