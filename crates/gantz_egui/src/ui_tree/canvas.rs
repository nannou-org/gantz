//! The renderer of the `canvas` element.
//!
//! Each panel of [`gantz_ui::figure`] draws as one plot area, stacked top to
//! bottom. Rects draw behind lines, segments and dots. Hovering a figure with
//! a tip shows it, the topmost figure first.

use super::plot::{PlotFrame, resolve_color, show_plot, stacked, y_bounds};
use egui_plot::{PlotPoint, PlotTransform};
use gantz_ui::Rgba;
use gantz_ui::figure::{Figure, Figures, Panel, Rect};

/// The stroke width of a segment, in points.
const SEGMENT_WIDTH: f32 = 2.0;
/// The radius of a dot, in points.
const DOT_RADIUS: f32 = 3.0;
/// How near the pointer must be to a segment or dot to hover it, in points.
const HOVER_DIST: f32 = 4.0;
/// The inset of a rect label and a title from their edges, in points.
const INSET: f32 = 3.0;
/// The fill and outline opacity of a rect.
const RECT_FILL: f32 = 0.2;
const RECT_STROKE: f32 = 0.6;

/// Draw the panels of `figs` filling `size`. Returns the union of the panel
/// responses.
pub(crate) fn canvas_body(
    frame: PlotFrame,
    figs: &Figures,
    id: egui::Id,
    size: egui::Vec2,
    ui: &mut egui::Ui,
) -> egui::Response {
    let empty = Panel::default();
    stacked(figs.panels.len(), size, ui, |i, sub_size, ui| {
        let panel = figs.panels.get(i).unwrap_or(&empty);
        panel_body(frame, panel, id.with(i), sub_size, ui)
    })
}

/// Draw one panel filling `size`.
fn panel_body(
    frame: PlotFrame,
    panel: &Panel,
    id: egui::Id,
    size: egui::Vec2,
    ui: &mut egui::Ui,
) -> egui::Response {
    let theme = resolve_color(None, ui);
    let color = move |c: Option<Rgba>| match c {
        Some(Rgba([r, g, b, a])) => egui::Color32::from_rgba_unmultiplied(r, g, b, a),
        None => theme,
    };
    let fit = |vals: Vec<f64>| y_bounds(vals, false, None, None);
    let figs = &panel.figures;
    let (xlo, xhi) = match panel.x {
        Some([lo, hi]) => (lo, hi.max(lo + f64::EPSILON)),
        None => fit(figs.iter().flat_map(Figure::xs).collect()),
    };
    let xpad = edge_pad(xlo, xhi, size.x, panel.pad);
    // A fitted y edge keeps the pad beyond the figures. A fixed edge stays
    // exact.
    let (flo, fhi) = fit(figs.iter().flat_map(Figure::ys).collect());
    let ypad = edge_pad(flo, fhi, size.y, panel.pad);
    let ylo = panel.y_min.unwrap_or(flo - ypad);
    let yhi = panel.y_max.unwrap_or(fhi + ypad).max(ylo + f64::EPSILON);
    let bounds = ([xlo - xpad, ylo], [xhi + xpad, yhi]);
    // The data space `y` values of a rect.
    let rect_ys = |r: &Rect| match r.frac {
        true => [ylo + r.min[1] * (yhi - ylo), ylo + r.max[1] * (yhi - ylo)],
        false => [r.min[1], r.max[1]],
    };

    let resp = show_plot(frame, id, size, bounds, ui, |plot_ui| {
        for f in figs {
            if let Figure::Rect(r) = f {
                let [y0, y1] = rect_ys(r);
                let c = color(r.color);
                let pts = vec![
                    [r.min[0], y0],
                    [r.max[0], y0],
                    [r.max[0], y1],
                    [r.min[0], y1],
                ];
                plot_ui.polygon(
                    egui_plot::Polygon::new("", pts)
                        .fill_color(c.gamma_multiply(RECT_FILL))
                        .stroke(egui::Stroke::new(1.0_f32, c.gamma_multiply(RECT_STROKE)))
                        .allow_hover(false),
                );
            }
        }
        for f in figs {
            match f {
                Figure::Seg(s) => plot_ui.line(
                    egui_plot::Line::new("", vec![s.from, s.to])
                        .color(color(s.color))
                        .width(SEGMENT_WIDTH)
                        .allow_hover(frame.interactive),
                ),
                Figure::Dot(d) => plot_ui.points(
                    egui_plot::Points::new("", vec![d.at])
                        .color(color(d.color))
                        .radius(DOT_RADIUS)
                        .filled(true)
                        .allow_hover(frame.interactive),
                ),
                Figure::Line(l) => plot_ui.line(
                    egui_plot::Line::new("", l.points.clone())
                        .color(color(l.color))
                        .allow_hover(frame.interactive),
                ),
                Figure::Rect(_) => (),
            }
        }
    });

    let transform = resp.transform;
    let clip = *transform.frame();
    let font = egui::TextStyle::Small.resolve(ui.style());
    for f in figs {
        let Figure::Rect(
            r @ Rect {
                label: Some(label), ..
            },
        ) = f
        else {
            continue;
        };
        let rect = screen_rect(&transform, [r.min[0], r.max[0]], rect_ys(r));
        ui.painter().with_clip_rect(rect.intersect(clip)).text(
            rect.left_center() + egui::vec2(INSET, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            font.clone(),
            color(r.color),
        );
    }
    if let Some(title) = &panel.title {
        ui.painter().with_clip_rect(clip).text(
            clip.left_top() + egui::vec2(INSET, INSET),
            egui::Align2::LEFT_TOP,
            title,
            font,
            ui.visuals().weak_text_color(),
        );
    }

    let response = resp.response;
    let tip = response
        .hover_pos()
        .and_then(|pos| hovered_tip(&transform, figs, &rect_ys, pos));
    match tip {
        Some(tip) => response.on_hover_ui_at_pointer(|ui| {
            ui.label(tip);
        }),
        None => response,
    }
}

/// The tip of the topmost figure under `pos`. Segments and dots draw over
/// rects, so they hover first.
fn hovered_tip(
    transform: &PlotTransform,
    figs: &[Figure],
    rect_ys: &dyn Fn(&Rect) -> [f64; 2],
    pos: egui::Pos2,
) -> Option<String> {
    let at = |p: [f64; 2]| transform.position_from_point(&PlotPoint::new(p[0], p[1]));
    let near_line = figs.iter().rev().find_map(|f| match f {
        Figure::Seg(s) => {
            let (a, b) = (at(s.from), at(s.to));
            let near = segment_dist(pos, a, b) <= HOVER_DIST;
            near.then(|| s.tip.clone()).flatten()
        }
        Figure::Dot(d) => {
            let near = at(d.at).distance(pos) <= DOT_RADIUS + HOVER_DIST;
            near.then(|| d.tip.clone()).flatten()
        }
        _ => None,
    });
    near_line.or_else(|| {
        figs.iter().rev().find_map(|f| match f {
            Figure::Rect(r) => {
                let rect = screen_rect(transform, [r.min[0], r.max[0]], rect_ys(r));
                rect.contains(pos).then(|| r.tip.clone()).flatten()
            }
            _ => None,
        })
    })
}

/// The distance in points from `p` to the segment from `a` to `b`.
fn segment_dist(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    let t = match len2 > 0.0 {
        true => ((p - a).dot(ab) / len2).clamp(0.0, 1.0),
        false => 0.0,
    };
    p.distance(a + ab * t)
}

/// The screen rect of the data space ranges `xs` and `ys`.
fn screen_rect(transform: &PlotTransform, xs: [f64; 2], ys: [f64; 2]) -> egui::Rect {
    let a = transform.position_from_point(&PlotPoint::new(xs[0], ys[0]));
    let b = transform.position_from_point(&PlotPoint::new(xs[1], ys[1]));
    egui::Rect::from_two_pos(a, b)
}

/// The padding in plot units, on each side of `lo..hi` drawn over `len`
/// points, that leaves `pad` points between the range and each edge. Zero
/// when `len` leaves no room for it.
fn edge_pad(lo: f64, hi: f64, len: f32, pad: f32) -> f64 {
    let room = f64::from(len) - 2.0 * f64::from(pad);
    match room > 0.0 {
        true => f64::from(pad) * (hi - lo) / room,
        false => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_segment_measures_from_its_nearest_point() {
        let (a, b) = (egui::pos2(0.0, 0.0), egui::pos2(10.0, 0.0));
        assert_eq!(segment_dist(egui::pos2(5.0, 3.0), a, b), 3.0);
        assert_eq!(segment_dist(egui::pos2(-4.0, 0.0), a, b), 4.0);
        assert_eq!(segment_dist(egui::pos2(1.0, 1.0), a, a), 2f32.sqrt());
    }

    #[test]
    fn edge_pad_leaves_the_points() {
        // 100 points over 0..1 with 5 points each side: 90 points for 1 unit.
        let pad = edge_pad(0.0, 1.0, 100.0, 5.0);
        assert!((pad - 5.0 / 90.0).abs() < 1e-12);
        assert_eq!(edge_pad(0.0, 1.0, 8.0, 5.0), 0.0, "no room");
    }
}
