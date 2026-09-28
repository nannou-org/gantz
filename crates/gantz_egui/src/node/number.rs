//! A number node with a dialer for editing its stored value.

use crate::{
    ContextMenuResponse, Env, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse,
    NodeViewResponse, SocketDoc, SocketKind, ui_tree::UiTree,
};
use gantz_core::node::{self, EvalConf, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use steel::SteelVal;

/// A number stored in state. Can be updated via the first input.
///
/// Optional configuration:
/// - `min` and `max` clamp every value, including input-socket values.
/// - `precision` controls how many decimals the dialer shows and edits. It
///   is display only.
/// - `push_eval_on_edit` toggles whether editing the dialer fires downstream.
///
/// Each field is serialized only when non-default. A plain `number` keeps
/// the original erased address. A configured field becomes part of the
/// node's identity, so it persists and is undoable under the
/// commit-on-change model.
#[derive(Clone, Debug, Serialize, Deserialize, NodeTag)]
pub struct Number {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    precision: Option<u8>,
    #[serde(
        default = "default_push_eval",
        skip_serializing_if = "is_default_push_eval"
    )]
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

    /// Set the lower bound. This affects the content address.
    pub fn set_min(&mut self, min: Option<f64>) {
        self.min = min;
    }

    /// Set the upper bound. This affects the content address.
    pub fn set_max(&mut self, max: Option<f64>) {
        self.max = max;
    }

    /// Set the dialer display precision. This is UI only.
    pub fn set_precision(&mut self, precision: Option<u8>) {
        self.precision = precision;
    }

    /// Set whether editing the dialer fires downstream. This is UI only.
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
        Hash::hash(&self.min.map(f64::to_bits), state);
        Hash::hash(&self.max.map(f64::to_bits), state);
        Hash::hash(&self.precision, state);
        Hash::hash(&self.push_eval_on_edit, state);
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
        node::parse_expr(&expr)
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        true
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        let path = ctx.path();
        let init = self.clamp(0.0);
        node::state::init_value_if_absent(ctx.vm(), path, || SteelVal::NumV(init)).unwrap()
    }
}

impl NodeUi for Number {
    fn name(&self, _: &Env<'_>) -> std::borrow::Cow<'_, str> {
        "number".into()
    }

    fn description(&self) -> Option<&'static str> {
        Some("A numeric value")
    }

    fn ui(&mut self, mut ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        // The numeric value lives in VM runtime state, not the node weight, so
        // editing the dialer never sets `changed`. The interpreter only queues
        // an evaluation when push is enabled.
        let frame = egui_graph::node::default_frame(uictx.style(), uictx.interaction());
        let (&id, prefix) = ctx.path().split_last().expect("a node path is never empty");
        let tree = fragment(self, id);
        let root_id = uictx.egui_id().with("gui");
        let mut payloads = Vec::new();
        let framed = uictx.framed_with(frame, |ui, _sockets| {
            let r = UiTree::new(root_id)
                .instance_prefix(prefix)
                .n_outputs(&|_: &[node::Id]| Some(1))
                .show(&tree, &mut ctx, ui);
            payloads = r.payloads;
            r.inner.unwrap_or_else(|| ui.response())
        });
        let mut resp = NodeUiResponse::new(framed);
        resp.payloads.extend(payloads);
        resp
    }

    fn view_ui(&mut self, mut ctx: NodeCtx, ui: &mut egui::Ui) -> NodeViewResponse {
        // The same fragment as the in-graph node. The pane provides the
        // background and margin.
        let (&id, prefix) = ctx.path().split_last().expect("a node path is never empty");
        let tree = fragment(self, id);
        let r = UiTree::new(ui.id().with("gui"))
            .instance_prefix(prefix)
            .n_outputs(&|_: &[node::Id]| Some(1))
            .show(&tree, &mut ctx, ui);
        let mut resp = NodeViewResponse::default();
        resp.inner = r.inner;
        resp.payloads = r.payloads;
        resp
    }

    fn inspector_rows(
        &mut self,
        ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        let row_h = crate::widget::node_inspector::table_row_h(body.ui_mut());
        // All four config fields contribute to the content address. `changed`
        // tracks any edit. `bounds_changed` also drives a re-clamp of the
        // stored value.
        let mut changed = false;
        let mut bounds_changed = false;

        // Min and max are two columns of one `range` row. Hover text says which
        // is which. They share the inspector's `bound_col` helper with the plot
        // node. Fixed-width dialers keep the max column put as the min value's
        // width changes.
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("range");
            });
            row.col(|ui| {
                egui::Grid::new("number_range")
                    .num_columns(2)
                    .show(ui, |ui| {
                        let mut min = self.min();
                        if crate::widget::node_inspector::bound_col(ui, "minimum", &mut min) {
                            self.set_min(min);
                            changed = true;
                            bounds_changed = true;
                        }
                        let mut max = self.max();
                        if crate::widget::node_inspector::bound_col(ui, "maximum", &mut max) {
                            self.set_max(max);
                            changed = true;
                            bounds_changed = true;
                        }
                        ui.end_row();
                    });
            });
        });

        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("prec.")
                    .on_hover_text("precision: decimal places the dialer shows (display only)");
            });
            row.col(|ui| {
                // The same checkbox and dialer widget as the `range` bounds, so
                // the rows look consistent.
                let mut on = self.precision().is_some();
                let mut n = self.precision().unwrap_or(2) as i32;
                let dialer = egui::DragValue::new(&mut n).range(0..=10).speed(0.1);
                let resp = ui
                    .add(
                        crate::widget::CheckboxEnabled::new(&mut on, dialer)
                            .width(crate::widget::node_inspector::DIAL_W),
                    )
                    .on_hover_text("precision: decimal places the dialer shows (display only)");
                if resp.changed() {
                    self.set_precision(on.then(|| n.clamp(0, 10) as u8));
                    changed = true;
                }
            });
        });

        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("push").on_hover_text(
                    "push-eval on edit: when enabled, editing the dialer fires a push \
                     evaluation downstream. Values arriving via the input socket are \
                     always passed through regardless.",
                );
            });
            row.col(|ui| {
                let mut push = self.push_eval_on_edit();
                if ui.checkbox(&mut push, "").changed() {
                    self.set_push_eval_on_edit(push);
                    changed = true;
                }
            });
        });

        let mut resp = InspectorRowsResponse::default();
        if changed {
            resp.mark_changed();
        }
        if bounds_changed {
            reclamp_stored(self, ctx, &mut resp);
        }
        resp
    }

    fn context_menu(&mut self, _ctx: &mut NodeCtx, ui: &mut egui::Ui) -> ContextMenuResponse {
        let mut resp = ContextMenuResponse::default();
        let mut push = self.push_eval_on_edit();
        if ui.checkbox(&mut push, "push-eval on edit").changed() {
            self.set_push_eval_on_edit(push);
            resp.mark_changed();
        }
        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        Some(match kind {
            SocketKind::Input => SocketDoc::ty("number").with_description(
                "new value to store. When unconnected the stored value is reused",
            ),
            SocketKind::Output => {
                SocketDoc::ty("number").with_description("the current stored value")
            }
        })
    }
}

fn default_push_eval() -> bool {
    true
}

fn is_default_push_eval(push_eval_on_edit: &bool) -> bool {
    *push_eval_on_edit == default_push_eval()
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

/// The number's dialer fragment, bound to its own state. Attrs come from the
/// weight, and the bind id is the node's id in its defining graph.
fn fragment(num: &Number, id: node::Id) -> gantz_ui::Element {
    gantz_ui::Element::Dialer(gantz_ui::Dialer {
        bind: Some(gantz_ui::BindPath(vec![id])),
        min: num.min(),
        max: num.max(),
        precision: num.precision(),
        push: num.push_eval_on_edit(),
        ..Default::default()
    })
}

/// Keep `max >= min` and re-clamp the stored value into the new bounds so the
/// displayed value, the stored state and the output stay consistent. Queues an
/// evaluation on `resp` when the value moved and push-eval is enabled.
fn reclamp_stored(num: &mut Number, ctx: &mut NodeCtx, resp: &mut InspectorRowsResponse) {
    if let (Some(lo), Some(hi)) = (num.min(), num.max()) {
        if hi < lo {
            num.set_max(Some(lo));
        }
    }
    if let Ok(Some(val)) = ctx.extract_value() {
        if let Some(clamped) = clamp_value(num, &val) {
            ctx.update_value(clamped).unwrap();
            if num.push_eval_on_edit() {
                resp.push_eval(ctx.path(), 1);
            }
        }
    }
}

/// The value clamped into `num`'s bounds, or `None` if it is already in range.
fn clamp_value(num: &Number, val: &SteelVal) -> Option<SteelVal> {
    match val {
        SteelVal::NumV(f) => {
            let c = num.clamp(*f);
            (c != *f).then_some(SteelVal::NumV(c))
        }
        SteelVal::IntV(i) => {
            let c = num.clamp(*i as f64);
            (c != *i as f64).then(|| {
                // Keep an integer when the clamp lands on a whole number.
                if c.fract() == 0.0 {
                    SteelVal::IntV(c as isize)
                } else {
                    SteelVal::NumV(c)
                }
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_core::{
        Edge, Node,
        compile::{EvalKind, entry_fn_name, push_pull_entrypoints},
        node::WithPushEval,
    };

    fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
        None
    }

    fn bounded(min: Option<f64>, max: Option<f64>) -> Number {
        let mut n = Number::default();
        n.set_min(min);
        n.set_max(max);
        n
    }

    // Push `value` into a `Number` configured with `min` and `max`. The `check`
    // expression asserts on the value forwarded downstream and panics on failure.
    // A successful fire proves the bounds were applied.
    fn assert_forwards(case: &str, value: &str, min: Option<f64>, max: Option<f64>, check: &str) {
        let mut g = petgraph::graph::DiGraph::new();
        let push =
            g.add_node(Box::new(node::expr(value).unwrap().with_push_eval()) as Box<dyn Node>);
        let num = g.add_node(Box::new(bounded(min, max)) as Box<_>);
        let check = g.add_node(Box::new(node::expr(check).unwrap()) as Box<_>);
        g.add_edge(push, num, Edge::from((0, 0)));
        g.add_edge(num, check, Edge::from((0, 0)));

        let config = gantz_core::compile::Config::default();
        let eps = push_pull_entrypoints(&no_lookup, &g);
        let (mut vm, _compiled) = gantz_core::vm::init(&no_lookup, &g, &eps, &config)
            .unwrap_or_else(|e| panic!("{case}: init: {}", gantz_core::vm::error_chain(&e)));

        let ep = eps
            .iter()
            .find(|ep| {
                ep.0.iter()
                    .any(|s| s.kind == EvalKind::Push && s.path == [push.index()])
            })
            .expect("push entrypoint");
        vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
            .unwrap_or_else(|e| panic!("{case}: firing the number errored: {e:?}"));
    }

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

    #[test]
    fn bounds_clamp_forwarded_values() {
        let rows: &[(&str, &str, Option<f64>, Option<f64>, &str)] = &[
            (
                "clamps_above_max",
                "150",
                Some(0.0),
                Some(100.0),
                "(assert! (= $n 100))",
            ),
            (
                "clamps_below_min",
                "-50",
                Some(0.0),
                Some(100.0),
                "(assert! (= $n 0))",
            ),
            (
                "passes_value_in_range",
                "42",
                Some(0.0),
                Some(100.0),
                "(assert! (= $n 42))",
            ),
            (
                "lower_bound_only",
                "-3",
                Some(0.0),
                None,
                "(assert! (= $n 0))",
            ),
            (
                "upper_bound_only",
                "500",
                None,
                Some(10.0),
                "(assert! (= $n 10))",
            ),
            (
                "unbounded_passes_through",
                "999",
                None,
                None,
                "(assert! (= $n 999))",
            ),
        ];
        for &(case, value, min, max, check) in rows {
            assert_forwards(case, value, min, max, check);
        }
    }

    #[test]
    fn fragment_bakes_weight_attrs_and_bind() {
        let mut num = Number::default();
        num.set_min(Some(0.0));
        num.set_max(Some(10.0));
        num.set_precision(Some(2));
        num.set_push_eval_on_edit(false);
        let expected = gantz_ui::Element::Dialer(gantz_ui::Dialer {
            bind: Some(gantz_ui::BindPath(vec![3])),
            min: Some(0.0),
            max: Some(10.0),
            precision: Some(2),
            push: false,
            ..Default::default()
        });
        assert_eq!(fragment(&num, 3), expected, "custom weight");

        let expected = gantz_ui::Element::Dialer(gantz_ui::Dialer {
            bind: Some(gantz_ui::BindPath(vec![0])),
            ..Default::default()
        });
        assert_eq!(
            fragment(&Number::default(), 0),
            expected,
            "default weight yields default dialer attrs"
        );
    }
}
