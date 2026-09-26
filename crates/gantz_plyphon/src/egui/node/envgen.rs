//! `~envgen`'s egui implementation.

use crate::egui::envelope::{View, envelope_editor};
use crate::egui::param::{params_state_row, rate_row};
use crate::envelope::{Envelope, Shape};
use crate::node::Envgen;
use crate::node::envgen::{SOCKETS, envelope_params};
use crate::param::{param_value_keyed, params_state, with_param_value};
use gantz_egui::node::PlotLook;
use gantz_egui::widget::node_inspector::table_row_h;
use gantz_egui::{
    ContextMenuResponse, Env, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse,
    NodeViewResponse, SocketDoc, SocketKind,
};
use std::borrow::Cow;

/// One frame of the editor over a node's envelope.
struct Edit {
    /// The response of the editor.
    response: egui::Response,
    /// The envelope of a drag in progress, for the running synth.
    live: Option<Envelope>,
    /// The envelope to commit to the node.
    commit: Option<Envelope>,
}

impl NodeUi for Envgen {
    fn name(&self, _: &Env<'_>) -> Cow<'_, str> {
        Cow::Borrowed("~envgen")
    }

    fn description(&self) -> Option<&'static str> {
        Some("Envelope generator, started and released by a gate")
    }

    fn ui(&mut self, mut ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        if self.compact() {
            let framed =
                uictx.framed(|ui, _sockets| ui.add(egui::Label::new("~envgen").selectable(false)));
            return NodeUiResponse::new(framed);
        }
        let id = uictx.egui_id().with("envgen");
        let [width, height] = self.size();
        let mut look = PlotLook {
            width,
            height,
            ..PlotLook::default()
        };
        let mut edit_out = None;
        let mut resp = look.body_ui(uictx, |ui, _look| {
            let size = ui.available_size();
            let edit = edit(ui, id, self, size);
            let response = edit.response.clone();
            edit_out = Some(edit);
            response
        });
        if let Some(edit) = edit_out {
            if apply(self, &mut ctx, edit) {
                resp.mark_changed();
            }
        }
        if [look.width, look.height] != self.size() {
            self.set_size([look.width, look.height]);
            resp.mark_changed();
        }
        resp
    }

    fn view_ui(&mut self, mut ctx: NodeCtx, ui: &mut egui::Ui) -> NodeViewResponse {
        // The view fills the pane and never writes back the body size. The id
        // derives from `ui`, which the caller scopes per pane.
        let id = ui.id().with("envgen");
        let size = ui.available_size();
        let edit = edit(ui, id, self, size);
        let mut out = NodeViewResponse::default();
        out.inner = Some(edit.response.clone());
        if apply(self, &mut ctx, edit) {
            out.mark_changed();
        }
        out
    }

    fn view_no_margin(&self) -> bool {
        true
    }

    fn context_menu(&mut self, _ctx: &mut NodeCtx, ui: &mut egui::Ui) -> ContextMenuResponse {
        let mut resp = ContextMenuResponse::default();
        let mut compact = self.compact();
        if ui
            .checkbox(&mut compact, "compact")
            .on_hover_text(COMPACT_DOC)
            .clicked()
        {
            self.set_compact(compact);
            resp.mark_changed();
            ui.close();
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
        let changed = display_row(body, self)
            | range_row(body, "x range", X_RANGE_DOC, self, Axis::X)
            | range_row(body, "y range", Y_RANGE_DOC, self, Axis::Y);
        if changed {
            resp.mark_changed();
        }
        // The envelope is node data, so each edit below is structural. The
        // inspector draws every node in one child ui, so popup ids carry the
        // node path.
        let path = ctx.path().to_vec();
        let mut env = self.envelope().clone();
        let edited = release_row(body, &path, &mut env) | segment_rows(body, &path, &mut env);
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

/// An axis of the plot.
#[derive(Clone, Copy)]
enum Axis {
    X,
    Y,
}

const COMPACT_DOC: &str = "show only the node name in the graph, not the editor";

const X_RANGE_DOC: &str = "the time range of the plot in seconds. It stays fixed while you edit";

const Y_RANGE_DOC: &str = "the level range of the plot. It stays fixed while you edit";

const RELEASE_DOC: &str = "where the envelope waits while the gate stays open. When the gate \
                           closes, the envelope plays on from here. With none, the envelope \
                           plays through without a wait";

/// The view of the editor for `node`.
fn view(node: &Envgen) -> View {
    View {
        grid: node.grid(),
        axes: node.axes(),
        x_range: node.x_range(),
        y_range: node.y_range(),
    }
}

/// Run one frame of the editor over the envelope of `node`. A drag edits a
/// copy in temp memory. The node data changes only when the drag ends, so a
/// drag is one undo step.
fn edit(ui: &mut egui::Ui, id: egui::Id, node: &Envgen, size: egui::Vec2) -> Edit {
    let drag_id = id.with("drag");
    let mut env = ui
        .data(|d| d.get_temp::<Envelope>(drag_id))
        .unwrap_or_else(|| node.envelope().clone());
    let editor = envelope_editor(ui, id, &mut env, size, view(node));
    let edits = editor.edits;
    let (live, commit) = if edits.drag_stopped || edits.edited {
        ui.data_mut(|d| d.remove::<Envelope>(drag_id));
        (None, Some(env))
    } else if edits.active {
        let live = edits.dragged.then(|| env.clone());
        ui.data_mut(|d| d.insert_temp(drag_id, env));
        (live, None)
    } else {
        ui.data_mut(|d| d.remove::<Envelope>(drag_id));
        (None, None)
    };
    Edit {
        response: editor.response,
        live,
        commit,
    }
}

/// Apply `edit` to `node`. Returns whether the node data changed.
fn apply(node: &mut Envgen, ctx: &mut NodeCtx, edit: Edit) -> bool {
    if let Some(env) = edit.live {
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
    match edit.commit {
        Some(env) => {
            node.set_envelope(env);
            true
        }
        None => false,
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

/// The grid, axes and compact toggles.
fn display_row(body: &mut egui_extras::TableBody, node: &mut Envgen) -> bool {
    let (mut grid, mut axes, mut compact) = (node.grid(), node.axes(), node.compact());
    let changed = row(body, "display", "how the node shows its envelope", |ui| {
        ui.horizontal(|ui| {
            let grid = ui
                .checkbox(&mut grid, "grid")
                .on_hover_text("draw the background grid");
            let axes = ui
                .checkbox(&mut axes, "axes")
                .on_hover_text("draw the time and level axes");
            let compact = ui
                .checkbox(&mut compact, "compact")
                .on_hover_text(COMPACT_DOC);
            grid.changed() | axes.changed() | compact.changed()
        })
        .inner
    });
    node.set_grid(grid);
    node.set_axes(axes);
    node.set_compact(compact);
    changed
}

/// The minimum and maximum of one plot axis.
fn range_row(
    body: &mut egui_extras::TableBody,
    name: &str,
    doc: &str,
    node: &mut Envgen,
    axis: Axis,
) -> bool {
    let [mut min, mut max] = match axis {
        Axis::X => node.x_range(),
        Axis::Y => node.y_range(),
    };
    let speed = ((max - min) * 0.005).max(0.0001);
    let changed = row(body, name, doc, |ui| {
        ui.horizontal(|ui| {
            let min_dv = egui::DragValue::new(&mut min)
                .speed(speed)
                .fixed_decimals(2);
            let max_dv = egui::DragValue::new(&mut max)
                .speed(speed)
                .fixed_decimals(2);
            let min_changed = ui.add(min_dv).on_hover_text("minimum").changed();
            let max_changed = ui.add(max_dv).on_hover_text("maximum").changed();
            min_changed | max_changed
        })
        .inner
    });
    if changed {
        match axis {
            Axis::X => node.set_x_range([min, max]),
            Axis::Y => node.set_y_range([min, max]),
        }
    }
    changed
}

/// The release point row.
fn release_row(body: &mut egui_extras::TableBody, path: &[usize], env: &mut Envelope) -> bool {
    let label = |release: Option<usize>| match release {
        None => "none".to_string(),
        Some(k) => format!("point {k}"),
    };
    row(body, "release", RELEASE_DOC, |ui| {
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
                    .fixed_decimals(2)
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
                let curve = egui::DragValue::new(&mut seg.curve)
                    .speed(0.05)
                    .fixed_decimals(2);
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
