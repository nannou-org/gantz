//! `~envgen`'s egui implementation.

use crate::egui::envelope::envelope_editor;
use crate::egui::param::{params_state_row, rate_row};
use crate::envelope::{Envelope, Shape};
use crate::node::Envgen;
use crate::node::envgen::{SOCKETS, envelope_params};
use crate::param::{param_value_keyed, params_state, with_param_value};
use gantz_egui::node::PlotLook;
use gantz_egui::ui_tree::plot::PlotFrame;
use gantz_egui::widget::node_inspector::table_row_h;
use gantz_egui::{
    Env, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind,
};
use std::borrow::Cow;

impl NodeUi for Envgen {
    fn name(&self, _: &Env<'_>) -> Cow<'_, str> {
        Cow::Borrowed("~envgen")
    }

    fn description(&self) -> Option<&'static str> {
        Some("Envelope generator, started and released by a gate")
    }

    fn ui(&mut self, mut ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let id = uictx.egui_id().with("envgen");
        let drag_id = id.with("drag");
        let [width, height] = self.size();
        let mut look = PlotLook {
            width,
            height,
            ..PlotLook::default()
        };
        let frame = PlotFrame {
            grid: self.grid(),
            axes: self.axes(),
            interactive: false,
        };
        let committed = self.envelope().clone();
        let mut live = None;
        let mut commit = None;
        let mut resp = look.body_ui(uictx, |ui, _look| {
            // A drag edits a copy in temp memory. The node data changes only
            // when the drag ends, so a drag is one undo step.
            let mut env = ui
                .data(|d| d.get_temp::<Envelope>(drag_id))
                .unwrap_or_else(|| committed.clone());
            let size = ui.available_size();
            let editor = envelope_editor(ui, id, &mut env, size, frame);
            let edits = editor.edits;
            if edits.drag_stopped || edits.edited {
                ui.data_mut(|d| d.remove::<Envelope>(drag_id));
                commit = Some(env);
            } else if edits.active {
                if edits.dragged {
                    live = Some(env.clone());
                }
                ui.data_mut(|d| d.insert_temp(drag_id, env));
            } else {
                ui.data_mut(|d| d.remove::<Envelope>(drag_id));
            }
            editor.response
        });
        if let Some(env) = live {
            // The running synth follows the drag. The commit at the end of the
            // drag carries the envelope to other peers, so this stays local.
            let prev = ctx
                .extract_value()
                .ok()
                .flatten()
                .unwrap_or_else(|| params_state(&[]));
            let state = envelope_params(&env)
                .into_iter()
                .fold(prev, |state, (name, value)| {
                    with_param_value(state, &name, value)
                });
            let _ = ctx.update_value_local(state);
        }
        if let Some(env) = commit {
            self.set_envelope(env);
            resp.mark_changed();
        }
        if [look.width, look.height] != self.size() {
            self.set_size([look.width, look.height]);
            resp.mark_changed();
        }
        resp
    }

    fn show_state(&self) -> bool {
        false
    }

    fn inspector_rows(
        &mut self,
        ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        let mut resp = InspectorRowsResponse::default();
        let state = ctx.extract_value().ok().flatten();
        params_state_row(body, state.as_ref());
        // The socket values live in VM state, so an edit is not structural.
        for socket in SOCKETS {
            let state = ctx.extract_value().ok().flatten();
            let mut value = state
                .as_ref()
                .and_then(|s| param_value_keyed(s, socket.name))
                .unwrap_or(socket.default as f64) as f32;
            let changed = row(body, socket.name, socket.doc, |ui| {
                ui.add(egui::DragValue::new(&mut value).speed(0.01))
                    .changed()
            });
            if changed {
                let prev = state.unwrap_or_else(|| params_state(&[]));
                let _ = ctx.update_value(with_param_value(prev, socket.name, value as f64));
            }
        }
        let mut rate = self.rate();
        if rate_row(body, &mut rate) {
            self.set_rate(rate);
            resp.mark_changed();
        }
        if display_row(body, self) {
            resp.mark_changed();
        }
        // The envelope is node data, so each edit below is structural. The
        // inspector draws every node in one child ui, so popup ids carry the
        // node path.
        let path = ctx.path().to_vec();
        let mut env = self.envelope().clone();
        let edited = preset_row(body, &mut env)
            | init_row(body, &mut env)
            | release_row(body, &path, &mut env)
            | segment_rows(body, &path, &mut env);
        if edited {
            self.set_envelope(env);
            resp.mark_changed();
        }
        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, ix: usize) -> Option<SocketDoc> {
        match kind {
            SocketKind::Input => {
                let socket = SOCKETS.get(ix)?;
                Some(SocketDoc::ty("signal | number").with_description(socket.doc))
            }
            SocketKind::Output => Some(SocketDoc::ty("signal").with_description("the envelope")),
        }
    }
}

/// One inspector row with the label `name`, which shows `doc` on hover, and
/// the `value` widgets. Returns what `value` returns.
fn row(
    body: &mut egui_extras::TableBody,
    name: &str,
    doc: &str,
    value: impl FnOnce(&mut egui::Ui) -> bool,
) -> bool {
    let mut changed = false;
    let row_h = table_row_h(body.ui_mut());
    body.row(row_h, |mut row| {
        row.col(|ui| {
            ui.label(name).on_hover_text(doc);
        });
        row.col(|ui| {
            changed = value(ui);
        });
    });
    changed
}

/// The grid and axes toggles of the body plot.
fn display_row(body: &mut egui_extras::TableBody, node: &mut Envgen) -> bool {
    let (mut grid, mut axes) = (node.grid(), node.axes());
    let changed = row(body, "display", "how the body plot looks", |ui| {
        ui.horizontal(|ui| {
            let grid_changed = ui
                .checkbox(&mut grid, "grid")
                .on_hover_text("draw the background grid")
                .changed();
            let axes_changed = ui
                .checkbox(&mut axes, "axes")
                .on_hover_text("draw the time and level axes")
                .changed();
            grid_changed | axes_changed
        })
        .inner
    });
    node.set_grid(grid);
    node.set_axes(axes);
    changed
}

/// A row of buttons that replace the envelope with a preset.
fn preset_row(body: &mut egui_extras::TableBody, env: &mut Envelope) -> bool {
    let presets = [
        (
            "perc",
            "rise to 1 and fall to 0, with no sustain",
            Envelope::perc(0.01, 1.0),
        ),
        (
            "adsr",
            "attack, decay, sustain at 0.5 and release",
            Envelope::adsr(0.01, 0.3, 0.5, 1.0),
        ),
        (
            "asr",
            "attack, sustain at 1 and release",
            Envelope::asr(0.01, 1.0, 1.0),
        ),
        (
            "triangle",
            "a straight rise to 1 and fall to 0",
            Envelope::triangle(1.0),
        ),
    ];
    row(body, "preset", "replace the envelope with a preset", |ui| {
        ui.horizontal(|ui| {
            let mut changed = false;
            for (name, doc, preset) in presets {
                if ui.small_button(name).on_hover_text(doc).clicked() {
                    *env = preset;
                    changed = true;
                }
            }
            changed
        })
        .inner
    })
}

/// The start level row.
fn init_row(body: &mut egui_extras::TableBody, env: &mut Envelope) -> bool {
    row(body, "start", "the level where the envelope starts", |ui| {
        ui.add(egui::DragValue::new(&mut env.init).speed(0.01))
            .changed()
    })
}

/// The release point row.
fn release_row(body: &mut egui_extras::TableBody, path: &[usize], env: &mut Envelope) -> bool {
    let label = |release: Option<usize>| match release {
        None => "none".to_string(),
        Some(k) => format!("point {k}"),
    };
    let doc = "where a held gate sustains. When the gate closes, the envelope \
               continues from here";
    row(body, "release", doc, |ui| {
        let mut changed = false;
        let n_points = env.n_points();
        egui::ComboBox::from_id_salt(("envgen-release", path))
            .selected_text(label(env.release))
            .show_ui(ui, |ui| {
                for option in std::iter::once(None).chain((1..n_points).map(Some)) {
                    changed |= ui
                        .selectable_value(&mut env.release, option, label(option))
                        .changed();
                }
            });
        changed
    })
}

/// One row per segment with its level, time, shape and curve.
fn segment_rows(body: &mut egui_extras::TableBody, path: &[usize], env: &mut Envelope) -> bool {
    let mut changed = false;
    for (i, seg) in env.segments.iter_mut().enumerate() {
        let name = format!("seg {i}");
        let doc = format!(
            "segment {i}: the level it ends at, its time, its shape and the \
             bend of a curve"
        );
        changed |= row(body, &name, &doc, |ui| {
            ui.horizontal(|ui| {
                let mut changed = false;
                let level = egui::DragValue::new(&mut seg.level).speed(0.01);
                changed |= ui.add(level).on_hover_text("level").changed();
                let time = egui::DragValue::new(&mut seg.time)
                    .range(0.0..=f32::MAX)
                    .speed(0.001)
                    .suffix(" s");
                changed |= ui.add(time).on_hover_text("time").changed();
                let width = ui.spacing().combo_width * 0.6;
                egui::ComboBox::from_id_salt(("envgen-shape", path, i))
                    .width(width)
                    .selected_text(seg.shape.label())
                    .show_ui(ui, |ui| {
                        for shape in Shape::ALL {
                            changed |= ui
                                .selectable_value(&mut seg.shape, shape, shape.label())
                                .changed();
                        }
                    });
                let curve = egui::DragValue::new(&mut seg.curve).speed(0.05);
                let enabled = seg.shape == Shape::Curve;
                changed |= ui
                    .add_enabled(enabled, curve)
                    .on_hover_text("curve")
                    .changed();
                changed
            })
            .inner
        });
    }
    changed
}
