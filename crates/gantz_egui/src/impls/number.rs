use crate::{
    ContextMenuResponse, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse, NodeViewResponse,
    Registry, SocketDoc, SocketKind,
};
use gantz_std::number::Number;
use steel::SteelVal;

impl NodeUi for Number {
    fn name(&self, _: &dyn Registry) -> &str {
        "number"
    }

    fn description(&self) -> Option<&'static str> {
        Some("A numeric value")
    }

    fn ui(&mut self, mut ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        // The numeric value lives in VM runtime state, not the node weight, so
        // editing the dialer does NOT change the graph's content address - we
        // only queue an evaluation (when enabled), never mark `changed`.
        let frame = egui_graph::node::default_frame(uictx.style(), uictx.interaction());
        let frame_fill = frame.fill;
        let mut do_eval = false;
        let framed = uictx.framed_with(frame, |ui, _sockets| {
            let (res, eval) = dialer_ui(self, &mut ctx, frame_fill, ui);
            do_eval = eval;
            res
        });
        let mut resp = NodeUiResponse::new(framed);
        if do_eval {
            resp.push_eval(ctx.path(), 1);
        }
        resp
    }

    fn view_ui(&mut self, mut ctx: NodeCtx, ui: &mut egui::Ui) -> NodeViewResponse {
        // The same dialer as the in-graph node; the pane provides the background
        // and margin. Editing updates VM state and (when enabled) queues an eval.
        let bg = ui.visuals().panel_fill;
        let (res, do_eval) = dialer_ui(self, &mut ctx, bg, ui);
        let mut resp = NodeViewResponse::default();
        resp.inner = Some(res);
        if do_eval {
            resp.push_eval(ctx.path(), 1);
        }
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

    fn socket_doc(&self, _: &dyn Registry, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        Some(match kind {
            SocketKind::Input => SocketDoc::ty("number")
                .with_description("new value to store; if unconnected the stored value is reused"),
            SocketKind::Output => {
                SocketDoc::ty("number").with_description("the current stored value")
            }
        })
    }
}

/// Render the value dialer (updating VM state on edit), shared by the in-graph
/// node body and the detached view. `bg` is the surrounding fill used to
/// "flatten" the dialer when push-eval is off - a cue that editing won't fire
/// downstream. Returns the dialer's response and whether a push-eval should fire
/// (an edit happened and push-eval is enabled).
fn dialer_ui(
    num: &Number,
    ctx: &mut NodeCtx,
    bg: egui::Color32,
    ui: &mut egui::Ui,
) -> (egui::Response, bool) {
    let push = num.push_eval_on_edit();
    if !push {
        let widgets = &mut ui.visuals_mut().widgets;
        widgets.inactive.weak_bg_fill = bg;
        widgets.inactive.bg_fill = bg;
    }
    let (min, max, precision) = (num.min(), num.max(), num.precision());
    let mut val = ctx.extract_value().unwrap().unwrap();
    let res = match val {
        SteelVal::NumV(ref mut f) => {
            let mut dv = egui::DragValue::new(f);
            if min.is_some() || max.is_some() {
                dv = dv.range(min.unwrap_or(f64::NEG_INFINITY)..=max.unwrap_or(f64::INFINITY));
            }
            if let Some(p) = precision {
                dv = dv.max_decimals(p as usize);
            }
            ui.add(dv)
        }
        SteelVal::IntV(ref mut i) => {
            let mut dv = egui::DragValue::new(i);
            if min.is_some() || max.is_some() {
                let lo = min.map_or(isize::MIN, |m| m as isize);
                let hi = max.map_or(isize::MAX, |m| m as isize);
                dv = dv.range(lo..=hi);
            }
            ui.add(dv)
        }
        _ => ui.add(egui::Label::new("ERR")),
    };
    let mut do_eval = false;
    if res.changed() {
        ctx.update_value(val).unwrap();
        if push {
            do_eval = true;
        }
    }
    (res, do_eval)
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
