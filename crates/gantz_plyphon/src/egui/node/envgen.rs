//! `~envgen`'s egui implementation.

use crate::egui::envelope::envelope_editor;
use crate::egui::param::{params_state_row, rate_row, value_row};
use crate::envelope::{Envelope, Shape};
use crate::node::Envgen;
use crate::node::envgen::{SOCKETS, envelope_params};
use crate::param::{param_value_keyed, params_state, with_param_value};
use gantz_egui::node::PlotLook;
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
            let editor = envelope_editor(ui, id, &mut env, size);
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
            let dv = egui::DragValue::new(&mut value).speed(0.01);
            if value_row(body, socket.name, dv) {
                let prev = state.unwrap_or_else(|| params_state(&[]));
                let _ = ctx.update_value(with_param_value(prev, socket.name, value as f64));
            }
        }
        let mut rate = self.rate();
        if rate_row(body, &mut rate) {
            self.set_rate(rate);
            resp.mark_changed();
        }
        // The envelope is node data, so each edit below is structural.
        let mut env = self.envelope().clone();
        let edited = preset_row(body, &mut env)
            | init_row(body, &mut env)
            | release_row(body, &mut env)
            | segment_rows(body, &mut env);
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

/// A row of buttons that replace the envelope with a preset.
fn preset_row(body: &mut egui_extras::TableBody, env: &mut Envelope) -> bool {
    let presets = [
        ("perc", Envelope::perc(0.01, 1.0)),
        ("adsr", Envelope::adsr(0.01, 0.3, 0.5, 1.0)),
        ("asr", Envelope::asr(0.01, 1.0, 1.0)),
        ("triangle", Envelope::triangle(1.0)),
    ];
    let mut changed = false;
    let row_h = table_row_h(body.ui_mut());
    body.row(row_h, |mut row| {
        row.col(|ui| {
            ui.label("preset");
        });
        row.col(|ui| {
            ui.horizontal(|ui| {
                for (name, preset) in presets {
                    if ui.small_button(name).clicked() {
                        *env = preset;
                        changed = true;
                    }
                }
            });
        });
    });
    changed
}

/// The start level row.
fn init_row(body: &mut egui_extras::TableBody, env: &mut Envelope) -> bool {
    let dv = egui::DragValue::new(&mut env.init).speed(0.01);
    value_row(body, "start", dv)
}

/// The release point row.
fn release_row(body: &mut egui_extras::TableBody, env: &mut Envelope) -> bool {
    let label = |release: Option<usize>| match release {
        None => "none".to_string(),
        Some(k) => format!("point {k}"),
    };
    let mut changed = false;
    let row_h = table_row_h(body.ui_mut());
    body.row(row_h, |mut row| {
        row.col(|ui| {
            ui.label("release");
        });
        row.col(|ui| {
            egui::ComboBox::from_id_salt("envgen-release")
                .selected_text(label(env.release))
                .show_ui(ui, |ui| {
                    let options = std::iter::once(None).chain((1..env.n_points()).map(Some));
                    for option in options {
                        changed |= ui
                            .selectable_value(&mut env.release, option, label(option))
                            .changed();
                    }
                });
        });
    });
    changed
}

/// One row per segment with its level, time, shape and curve.
fn segment_rows(body: &mut egui_extras::TableBody, env: &mut Envelope) -> bool {
    let mut changed = false;
    for (i, seg) in env.segments.iter_mut().enumerate() {
        let row_h = table_row_h(body.ui_mut());
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label(format!("segment {i}"));
            });
            row.col(|ui| {
                ui.horizontal(|ui| {
                    let level = egui::DragValue::new(&mut seg.level).speed(0.01);
                    changed |= ui.add(level).on_hover_text("level").changed();
                    let time = egui::DragValue::new(&mut seg.time)
                        .range(0.0..=f32::MAX)
                        .speed(0.001)
                        .suffix(" s");
                    changed |= ui.add(time).on_hover_text("time").changed();
                    egui::ComboBox::from_id_salt(("envgen-shape", i))
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
                });
            });
        });
    }
    changed
}
