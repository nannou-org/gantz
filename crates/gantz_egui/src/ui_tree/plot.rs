//! The shared plot leaf renderer.
//!
//! Draws per-channel numeric series with `egui_plot`. [`PlotParams`]
//! parameterizes it, so the same code renders both the `Plot` node and the
//! interpreter's `plot` element. The node takes its params from its weight.
//! The element takes them from its attrs. [`show_plot`] is the plot area
//! beneath both, for plot-like nodes that draw their own items.

use steel::SteelVal;

/// The plot area's grid, axes and hover behaviour.
#[derive(Clone, Copy, Debug)]
pub struct PlotFrame {
    /// Whether a grid draws behind the data.
    pub grid: bool,
    /// Whether axes draw.
    pub axes: bool,
    /// Whether hovering the data shows a crosshair and value readout.
    pub interactive: bool,
}

/// The resolved rendering parameters of one plot.
pub(crate) struct PlotParams {
    /// How the series is drawn.
    pub style: gantz_ui::PlotStyle,
    /// Plot colour, theme default when absent.
    pub color: Option<[u8; 4]>,
    /// The plot area.
    pub frame: PlotFrame,
    /// A fixed lower value axis bound.
    pub y_min: Option<f32>,
    /// A fixed upper value axis bound.
    pub y_max: Option<f32>,
}

/// Render the plot filling `size`. A single channel fills it. Multiple channels
/// are stacked as one sub-plot each. Returns the combined response.
pub(crate) fn plot_body(
    params: &PlotParams,
    channels: &[Vec<f64>],
    plot_id: egui::Id,
    size: egui::Vec2,
    ui: &mut egui::Ui,
) -> egui::Response {
    // Zero or one channel draws a single plot. An empty plot keeps the node's
    // body visible.
    if channels.len() <= 1 {
        let ys = channels.first().map(Vec::as_slice).unwrap_or(&[]);
        return plot_channel(params, ys, plot_id, size, ui);
    }
    stacked(channels.len(), size, ui, |i, sub_size, ui| {
        plot_channel(params, &channels[i], plot_id.with(i), sub_size, ui)
    })
}

/// Stack `n` rows vertically, splitting the height of `size` evenly. `row`
/// draws row `i` filling the given size. Returns the union of the row
/// responses. With `n` of zero, one row is drawn.
///
/// The rows and the item spacing between them never exceed `size`. A
/// `Resize` parent grows to fit content larger than itself, so any excess
/// would grow the node body on every frame.
pub fn stacked(
    n: usize,
    size: egui::Vec2,
    ui: &mut egui::Ui,
    mut row: impl FnMut(usize, egui::Vec2, &mut egui::Ui) -> egui::Response,
) -> egui::Response {
    let n = n.max(1);
    let gaps = ui.spacing().item_spacing.y * (n - 1) as f32;
    let row_h = ((size.y - gaps) / n as f32).floor().max(0.0);
    let sub_size = egui::vec2(size.x, row_h);
    ui.vertical(|ui| {
        (0..n)
            .map(|i| row(i, sub_size, ui))
            .reduce(|a, b| a.union(b))
            .expect("at least one row")
    })
    .inner
}

/// Render one channel's series filling `size`, with its axes, grid, line or bars
/// and bounds.
fn plot_channel(
    params: &PlotParams,
    ys: &[f64],
    plot_id: egui::Id,
    size: egui::Vec2,
    ui: &mut egui::Ui,
) -> egui::Response {
    let color = resolve_color(params.color, ui);
    let interactive = params.frame.interactive;
    let bounds = value_bounds(ys, params.style, params.y_min, params.y_max);
    show_plot(
        params.frame,
        plot_id,
        size,
        bounds,
        ui,
        |plot_ui| match params.style {
            gantz_ui::PlotStyle::Bars => {
                let bars = ys
                    .iter()
                    .enumerate()
                    .map(|(i, &y)| {
                        egui_plot::Bar::new(i as f64, y)
                            .width(1.0)
                            .fill(color)
                            .stroke(egui::Stroke::NONE)
                    })
                    .collect();
                plot_ui.bar_chart(egui_plot::BarChart::new("", bars).allow_hover(interactive));
            }
            gantz_ui::PlotStyle::Line => {
                let points = egui_plot::PlotPoints::from_ys_f64(ys);
                plot_ui.line(
                    egui_plot::Line::new("", points)
                        .color(color)
                        .allow_hover(interactive),
                );
            }
        },
    )
    .response
}

/// Render a plot area filling `size` with the view fixed to `bounds`, given as
/// `([x_min, y_min], [x_max, y_max])`. `draw` adds the plot items. Items should
/// pass `frame.interactive` to their `allow_hover`, so a non-interactive plot
/// shows no value readout.
///
/// Pan and zoom are always off. The plot senses hover only, so the node frame
/// beneath still captures drags and right-clicks.
pub fn show_plot(
    frame: PlotFrame,
    plot_id: egui::Id,
    size: egui::Vec2,
    bounds: ([f64; 2], [f64; 2]),
    ui: &mut egui::Ui,
    draw: impl FnOnce(&mut egui_plot::PlotUi),
) -> egui_plot::PlotResponse<()> {
    let mut plot = egui_plot::Plot::new(plot_id)
        .width(size.x)
        .height(size.y)
        .show_background(false)
        .show_axes(egui::Vec2b::new(frame.axes, frame.axes))
        .show_grid(egui::Vec2b::new(frame.grid, frame.grid))
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .sense(egui::Sense::hover());
    if !frame.interactive {
        plot = plot.cursor_color(egui::Color32::TRANSPARENT);
    }

    let plot_resp = plot.show(ui, |plot_ui| {
        draw(plot_ui);
        // Drive the view from the data and config. The plot never pans,
        // so live updates and min/max apply.
        let ([xlo, ylo], [xhi, yhi]) = bounds;
        plot_ui.set_plot_bounds_x(xlo..=xhi);
        plot_ui.set_plot_bounds_y(ylo..=yhi);
    });

    // egui_plot sets a crosshair mouse cursor on hover. When not interactive,
    // restore the default arrow so the plot reads as a static node. The
    // resize corner sets its own cursor after this, so it is unaffected.
    if !frame.interactive && plot_resp.response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
    }
    plot_resp
}

/// Compute `([x_min, y_min], [x_max, y_max])` for the view from the data and
/// optional fixed value bounds. Bars include the baseline `0` and span integer
/// x. Lines span sample indices. The plot itself adds no margin.
fn value_bounds(
    ys: &[f64],
    style: gantz_ui::PlotStyle,
    y_min: Option<f32>,
    y_max: Option<f32>,
) -> ([f64; 2], [f64; 2]) {
    let n = ys.len() as f64;
    let (xlo, xhi, baseline) = match style {
        gantz_ui::PlotStyle::Bars => (-0.5, (n - 0.5).max(0.5), true),
        gantz_ui::PlotStyle::Line => (0.0, (n - 1.0).max(1.0), false),
    };
    let (ylo, yhi) = y_bounds(ys.iter().copied(), baseline, y_min, y_max);
    ([xlo, ylo], [xhi, yhi])
}

/// Compute `(y_min, y_max)` for the view from `values` and optional fixed
/// bounds. With `baseline`, `0` stays in view. A flat range is padded by `1`
/// either side. Non-finite values are ignored. No values give `0..1`. Fixed
/// bounds replace the computed ones.
pub fn y_bounds(
    values: impl IntoIterator<Item = f64>,
    baseline: bool,
    y_min: Option<f32>,
    y_max: Option<f32>,
) -> (f64, f64) {
    let (dmin, dmax) = values
        .into_iter()
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(v), hi.max(v))
        });
    let (mut ylo, mut yhi) = match (dmin <= dmax, baseline) {
        (true, true) => (dmin.min(0.0), dmax.max(0.0)),
        (true, false) => (dmin, dmax),
        (false, _) => (0.0, 1.0),
    };
    if (yhi - ylo).abs() < 1e-9 {
        ylo -= 1.0;
        yhi += 1.0;
    }
    (y_min.map_or(ylo, f64::from), y_max.map_or(yhi, f64::from))
}

/// Resolve the configured colour, falling back to the theme's strong text
/// colour when unset.
pub fn resolve_color(color: Option<[u8; 4]>, ui: &egui::Ui) -> egui::Color32 {
    match color {
        Some([r, g, b, a]) => egui::Color32::from_rgba_unmultiplied(r, g, b, a),
        None => ui.visuals().strong_text_color(),
    }
}

/// Split a stored plot value into per-channel series. A list or vector of
/// containers is one series per inner container. A flat numeric list or vector
/// is a single channel. So is a lone number. [`SteelVal::ListV`] and
/// [`SteelVal::VectorV`] are treated identically.
pub(crate) fn split_channels(val: &SteelVal) -> Vec<Vec<f64>> {
    // The top-level elements of a list or vector. `None` if `val` is not a container.
    let elems: Option<Vec<&SteelVal>> = match val {
        SteelVal::ListV(list) => Some(list.iter().collect()),
        SteelVal::VectorV(vec) => Some(vec.iter().collect()),
        _ => None,
    };
    match elems {
        // Containers nested in a container give one series each.
        Some(elems) if elems.iter().any(|v| is_container(v)) => {
            elems.iter().map(|v| channel_numerics(v)).collect()
        }
        // A flat numeric container is a single channel.
        Some(elems) => vec![elems.iter().filter_map(|v| steel_num(v)).collect()],
        // A lone number is one single-sample channel.
        None => vec![steel_num(val).into_iter().collect()],
    }
}

/// Whether `v` is a list or vector, the channel container shapes.
pub(crate) fn is_container(v: &SteelVal) -> bool {
    matches!(v, SteelVal::ListV(_) | SteelVal::VectorV(_))
}

/// One channel's numeric samples. A list's or vector's numeric elements, or a lone number.
fn channel_numerics(val: &SteelVal) -> Vec<f64> {
    match val {
        SteelVal::ListV(list) => list.iter().filter_map(steel_num).collect(),
        SteelVal::VectorV(vec) => vec.iter().filter_map(steel_num).collect(),
        other => steel_num(other).into_iter().collect(),
    }
}

/// Convert a numeric [`SteelVal`] to `f64`.
pub fn steel_num(val: &SteelVal) -> Option<f64> {
    match val {
        SteelVal::NumV(f) => Some(*f),
        SteelVal::IntV(i) => Some(*i as f64),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_channels_by_shape() {
        let num = |n: f64| SteelVal::NumV(n);
        let list = |xs: Vec<SteelVal>| SteelVal::ListV(xs.into_iter().collect());
        let vector = |xs: Vec<SteelVal>| SteelVal::VectorV(xs.into_iter().collect());

        // A flat numeric list or vector is one channel.
        assert_eq!(
            split_channels(&list(vec![num(1.0), num(2.0), num(3.0)])),
            vec![vec![1.0, 2.0, 3.0]],
        );
        assert_eq!(
            split_channels(&vector(vec![num(1.0), num(2.0), num(3.0)])),
            vec![vec![1.0, 2.0, 3.0]],
        );
        // A lone number is one single-sample channel.
        assert_eq!(split_channels(&num(7.0)), vec![vec![7.0]]);
        // A list of lists, a vector of vectors, and a mixed list of vectors all give
        // one channel per inner container.
        let expected = vec![vec![1.0, 3.0], vec![2.0, 4.0]];
        assert_eq!(
            split_channels(&list(vec![
                list(vec![num(1.0), num(3.0)]),
                list(vec![num(2.0), num(4.0)]),
            ])),
            expected,
        );
        assert_eq!(
            split_channels(&vector(vec![
                vector(vec![num(1.0), num(3.0)]),
                vector(vec![num(2.0), num(4.0)]),
            ])),
            expected,
        );
        assert_eq!(
            split_channels(&list(vec![
                vector(vec![num(1.0), num(3.0)]),
                vector(vec![num(2.0), num(4.0)]),
            ])),
            expected,
        );
    }

    // Stacked rows and the spacing between them fit within the given height,
    // so a resizable parent never grows to fit them.
    #[test]
    fn stacked_fits_height() {
        let ctx = egui::Context::default();
        let size = egui::vec2(100.0, 101.0);
        for n in 0..=6 {
            let mut used = egui::Vec2::ZERO;
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    used = ui
                        .scope(|ui| {
                            stacked(n, size, ui, |_, s, ui| {
                                ui.allocate_exact_size(s, egui::Sense::hover()).1
                            })
                        })
                        .response
                        .rect
                        .size();
                });
            });
            assert!(used.y <= size.y, "{n} rows use {} of {}", used.y, size.y);
        }
    }

    #[test]
    fn value_bounds_by_style() {
        use gantz_ui::PlotStyle::{Bars, Line};

        // Bars span integer bar positions on x and include the baseline 0 on y.
        let ([xlo, ylo], [xhi, yhi]) = value_bounds(&[1.0, 2.0, 3.0], Bars, None, None);
        assert_eq!((xlo, xhi), (-0.5, 2.5));
        assert_eq!((ylo, yhi), (0.0, 3.0));

        // Lines span sample indices on x and the data on y.
        let ([xlo, ylo], [xhi, yhi]) = value_bounds(&[1.0, 2.0, 3.0], Line, None, None);
        assert_eq!((xlo, xhi), (0.0, 2.0));
        assert_eq!((ylo, yhi), (1.0, 3.0));

        // A flat series is padded so it stays visible.
        let ([_, ylo], [_, yhi]) = value_bounds(&[2.0, 2.0], Line, None, None);
        assert_eq!((ylo, yhi), (1.0, 3.0));

        // Fixed overrides are exact.
        let ([_, ylo], [_, yhi]) = value_bounds(&[1.0, 2.0], Line, Some(-1.0), Some(1.0));
        assert_eq!((ylo, yhi), (-1.0, 1.0));

        // Non-finite values do not affect the fit.
        let ([_, ylo], [_, yhi]) =
            value_bounds(&[1.0, f64::INFINITY, f64::NAN, 3.0], Line, None, None);
        assert_eq!((ylo, yhi), (1.0, 3.0));

        // No data gives a unit default window.
        let ([xlo, ylo], [xhi, yhi]) = value_bounds(&[], Bars, None, None);
        assert_eq!((xlo, xhi), (-0.5, 0.5));
        assert_eq!((ylo, yhi), (0.0, 1.0));
    }
}
