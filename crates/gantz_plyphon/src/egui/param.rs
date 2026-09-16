//! Shared inspector rows for DSP node parameters. The rows read and write
//! through the headless param helpers in [`crate::param`].

use crate::param::{pending_len, pending_len_total};
use gantz_core::steel::SteelVal;

/// One inspector row toggling a node's ugen rate between `ar` and `kr`.
/// Returns `true` on change. The change is structural because the emitted
/// unit's rate changes, so the caller must `mark_changed`.
pub fn rate_row(body: &mut egui_extras::TableBody, rate: &mut crate::dsp::NodeRate) -> bool {
    use crate::dsp::NodeRate;
    let row_h = gantz_egui::widget::node_inspector::table_row_h(body.ui_mut());
    let mut changed = false;
    body.row(row_h, |mut row| {
        row.col(|ui| {
            ui.label("rate");
        });
        row.col(|ui| {
            ui.horizontal(|ui| {
                changed |= gantz_egui::widget::node_inspector::radio_option(
                    ui,
                    rate,
                    NodeRate::Audio,
                    "ar",
                    "audio rate: one value per sample",
                );
                changed |= gantz_egui::widget::node_inspector::radio_option(
                    ui,
                    rate,
                    NodeRate::Control,
                    "kr",
                    "control rate: one value per block (cheaper, for modulators - \
                     audio sinks lift it back to audio)",
                );
            });
        });
    });
    changed
}

/// The fixed width in pixels of a controllable param's value dialer. The
/// smoothing `lag` dialer to its right then stays put as the value's text width
/// changes while dragging. It mirrors `node_inspector::DIAL_W` but is wider to
/// fit a frequency like `20000 Hz`.
const VALUE_W: f32 = 80.0;

/// One inspector row for a DSP param. The table key column is `name`. The value
/// column shows the caller-configured `value` dialer at a fixed width with the
/// smoothing `lag` to its right. For example `0.234   0.010 s lag`.
///
/// Returns `(value_changed, lag_changed)`. The caller writes the value to VM node
/// state without `mark_changed`, since it is not content-addressed. The caller
/// writes the lag to the node weight with `mark_changed`, since it is structural.
pub fn param_row(
    body: &mut egui_extras::TableBody,
    name: &str,
    value: egui::DragValue<'_>,
    lag: &mut f32,
) -> (bool, bool) {
    let row_h = gantz_egui::widget::node_inspector::table_row_h(body.ui_mut());
    let mut value_changed = false;
    let mut lag_changed = false;
    body.row(row_h, |mut row| {
        row.col(|ui| {
            ui.label(name);
        });
        row.col(|ui| {
            ui.horizontal(|ui| {
                // Fixed width so the following lag dialer does not flicker as the
                // value's width changes while dragging.
                let value_h = ui.spacing().interact_size.y;
                value_changed = ui.add_sized([VALUE_W, value_h], value).changed();
                let lag_dv = egui::DragValue::new(lag)
                    .range(0.0..=10.0)
                    .speed(0.001)
                    .fixed_decimals(3)
                    .suffix(" s lag");
                lag_changed = ui
                    .add(lag_dv)
                    .on_hover_text("one-pole smoothing time in seconds (0 = instant)")
                    .changed();
            });
        });
    });
    (value_changed, lag_changed)
}

/// One inspector row for a single value with no smoothing-lag field. The key
/// column is `name` and the value column is the caller-configured `value`
/// dialer. Returns whether it changed. Used for params that are themselves a
/// duration, such as the `~lag` lag time, where nothing is smoothed.
pub fn value_row(
    body: &mut egui_extras::TableBody,
    name: &str,
    value: egui::DragValue<'_>,
) -> bool {
    let row_h = gantz_egui::widget::node_inspector::table_row_h(body.ui_mut());
    let mut changed = false;
    body.row(row_h, |mut row| {
        row.col(|ui| {
            ui.label(name);
        });
        row.col(|ui| {
            changed = ui.add(value).changed();
        });
    });
    changed
}

/// One inspector row summarising a DSP node's queued control updates. The key
/// column is `"state"` and the value column shows `"{n} queued"`. That is the
/// number of scheduled but not yet drained control updates across the node's
/// params.
///
/// DSP nodes return `false` from `show_state` and call this instead, in place
/// of the inspector's default raw `{value, pending}` state dump. This row reads
/// the bare single-param state shape. Keyed multi-param nodes use
/// [`params_state_row`].
pub fn param_state_row(body: &mut egui_extras::TableBody, state: Option<&SteelVal>) {
    queued_row(body, state.map(pending_len).unwrap_or(0));
}

/// The [`param_state_row`] analogue for a node with keyed multi-param state.
/// The queued count sums every param's pending queue.
pub fn params_state_row(body: &mut egui_extras::TableBody, state: Option<&SteelVal>) {
    queued_row(body, state.map(pending_len_total).unwrap_or(0));
}

fn queued_row(body: &mut egui_extras::TableBody, n: usize) {
    let row_h = gantz_egui::widget::node_inspector::table_row_h(body.ui_mut());
    body.row(row_h, |mut row| {
        row.col(|ui| {
            ui.label("state");
        });
        row.col(|ui| {
            ui.label(format!("{n} queued"))
                .on_hover_text("pending scheduled control updates");
        });
    });
}
