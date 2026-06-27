use crate::{
    ContextMenuResponse, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse, Registry,
    SocketDoc, SocketKind,
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
        let push = self.push_eval_on_edit();
        let (min, max, precision) = (self.min(), self.max(), self.precision());
        let mut do_eval = false;
        let framed = uictx.framed_with(frame, |ui, _sockets| {
            // When push-eval is disabled, flatten the dialer so it merges with
            // the node frame - a cue that editing won't fire downstream.
            if !push {
                let widgets = &mut ui.visuals_mut().widgets;
                widgets.inactive.weak_bg_fill = frame_fill;
                widgets.inactive.bg_fill = frame_fill;
            }
            let mut val = ctx.extract_value().unwrap().unwrap();
            let res = match val {
                SteelVal::NumV(ref mut f) => {
                    let mut dv = egui::DragValue::new(f);
                    if min.is_some() || max.is_some() {
                        dv = dv
                            .range(min.unwrap_or(f64::NEG_INFINITY)..=max.unwrap_or(f64::INFINITY));
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
            if res.changed() {
                ctx.update_value(val).unwrap();
                if push {
                    do_eval = true;
                }
            }
            res
        });
        let mut resp = NodeUiResponse::new(framed);
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

        // Min and max share a `range` row. An inner grid with fixed-width
        // dialers keeps the `max` group's position fixed regardless of the
        // `min` value's width.
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("range").on_hover_text(
                    "optional min/max bounds; every value is clamped into this range \
                     (input-socket values included)",
                );
            });
            row.col(|ui| {
                egui::Grid::new(ui.id().with("range"))
                    .num_columns(4)
                    .spacing([6.0, 4.0])
                    .show(ui, |ui| {
                        let (new_min, min_changed) = bound_cells(ui, "mn", "minimum", self.min());
                        if min_changed {
                            self.set_min(new_min);
                            changed = true;
                            bounds_changed = true;
                        }
                        let (new_max, max_changed) = bound_cells(ui, "mx", "maximum", self.max());
                        if max_changed {
                            self.set_max(new_max);
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
                ui.horizontal(|ui| {
                    let mut enabled = self.precision().is_some();
                    if ui.checkbox(&mut enabled, "").changed() {
                        self.set_precision(enabled.then(|| self.precision().unwrap_or(2)));
                        changed = true;
                    }
                    let mut n = self.precision().unwrap_or(2) as i32;
                    if ui
                        .add_enabled(
                            enabled,
                            egui::DragValue::new(&mut n).range(0..=10).speed(0.1),
                        )
                        .changed()
                    {
                        self.set_precision(Some(n.clamp(0, 10) as u8));
                        changed = true;
                    }
                });
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

/// Fixed dialer width (px) so a bound's position does not shift with its value.
const BOUND_DIAL_W: f32 = 48.0;

/// Render one bound as a `label` grid cell followed by a `<checkbox> <dialer>`
/// grid cell. `hover` expands the abbreviated label.
///
/// The dialer is always shown but disabled while the checkbox is off, and is
/// given a fixed width so the next grid column stays put. The checkbox and
/// dialer are packed tightly. Returns the (possibly updated) bound and whether
/// it changed this frame.
fn bound_cells(
    ui: &mut egui::Ui,
    label: &str,
    hover: &str,
    value: Option<f64>,
) -> (Option<f64>, bool) {
    ui.label(label).on_hover_text(hover);
    let mut value = value;
    let changed = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x *= 0.25;
            let mut changed = false;
            let mut enabled = value.is_some();
            if ui.checkbox(&mut enabled, "").changed() {
                value = enabled.then(|| value.unwrap_or(0.0));
                changed = true;
            }
            let mut v = value.unwrap_or(0.0);
            let res = ui
                .add_enabled_ui(enabled, |ui| {
                    let size = [BOUND_DIAL_W, ui.spacing().interact_size.y];
                    ui.add_sized(size, egui::DragValue::new(&mut v).speed(0.1))
                })
                .inner;
            if res.changed() {
                value = Some(v);
                changed = true;
            }
            changed
        })
        .inner;
    (value, changed)
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
