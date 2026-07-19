use crate::{
    ContextMenuResponse, Env, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse,
    NodeViewResponse, SocketDoc, SocketKind, node, ui_tree::UiTree,
};
use gantz_std::number::Number;
use steel::SteelVal;

impl NodeUi for Number {
    fn name(&self, _: &Env<'_>) -> std::borrow::Cow<'_, str> {
        "number".into()
    }

    fn description(&self) -> Option<&'static str> {
        Some("A numeric value")
    }

    fn ui(&mut self, mut ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        // The numeric value lives in VM runtime state, not the node weight, so
        // editing the dialer does NOT change the graph's content address - the
        // interpreter only queues an evaluation (when enabled), never `changed`.
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
        // The same fragment as the in-graph node; the pane provides the
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
        // All four config fields contribute to the content address (so they
        // persist and are undoable): `changed` tracks any edit; `bounds_changed`
        // additionally drives a re-clamp of the stored value.
        let mut changed = false;
        let mut bounds_changed = false;

        // Min and max are two columns of one `range` row (hover text says which
        // is which), sharing the inspector's `bound_col` helper with the plot
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
                // Same checkbox+dialer widget as the `range` bounds, so the rows
                // look consistent.
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
            SocketKind::Input => SocketDoc::ty("number")
                .with_description("new value to store; if unconnected the stored value is reused"),
            SocketKind::Output => {
                SocketDoc::ty("number").with_description("the current stored value")
            }
        })
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
/// evaluation on `resp` when the value moved (and push-eval is enabled).
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
        assert_eq!(fragment(&num, 3), expected);
    }

    #[test]
    fn default_weight_yields_default_dialer_attrs() {
        let expected = gantz_ui::Element::Dialer(gantz_ui::Dialer {
            bind: Some(gantz_ui::BindPath(vec![0])),
            ..Default::default()
        });
        assert_eq!(fragment(&Number::default(), 0), expected);
    }
}
