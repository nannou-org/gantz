//! Laying out and drawing decoded plot data.
//!
//! Within one plot, numeric events draw on the value axis as segments with
//! onset dots, and numeric signals as lines. Non-numeric events draw as boxes
//! in lanes that divide the plot height, behind the numeric data. Label boxes
//! show their text clipped within the box. Hovering any event shows its value.

use super::data::{Channel, ChannelKey, Leaf, PlotData};
use super::{KeyColor, ValueLayout};
use egui_plot::{PlotPoint, PlotTransform};
use gantz_egui::node::PlotLook;
use gantz_egui::ui_tree::plot::{resolve_color, show_plot, stacked, y_bounds};

/// How a plot draws, from the node's weight.
#[derive(Clone, Copy)]
pub(super) struct DrawConf<'a> {
    pub look: &'a PlotLook,
    pub layout: ValueLayout,
    pub key_colors: &'a [KeyColor],
}

/// A non-numeric event's box, placed in a lane.
struct LaneBox<'a> {
    key: &'a ChannelKey,
    leaf: &'a Leaf,
    start: f64,
    end: f64,
    /// The lane index from the top.
    lane: usize,
    color: egui::Color32,
}

/// The stroke width of an event segment, in points.
const SEGMENT_WIDTH: f32 = 2.0;
/// The radius of an onset dot, in points.
const ONSET_RADIUS: f32 = 3.0;
/// The space kept between the data and each fitted plot edge, in points.
/// It fits an onset dot plus a point of anti-aliasing.
const EDGE_PAD: f32 = ONSET_RADIUS + 1.0;
/// How near the pointer must be to a segment to hover it, in points.
const HOVER_DIST: f32 = 4.0;
/// The fraction of a lane's height left as a gap above and below its boxes.
const LANE_GAP: f64 = 0.1;
/// The inset of a box label from the box's left edge, in points.
const LABEL_INSET: f32 = 3.0;

/// Draw the plot data filling `size`.
pub(super) fn draw(
    conf: DrawConf<'_>,
    data: &PlotData,
    plot_id: egui::Id,
    size: egui::Vec2,
    ui: &mut egui::Ui,
) -> egui::Response {
    let chans: Vec<(&ChannelKey, &Channel)> = data.channels.iter().collect();
    match conf.layout {
        ValueLayout::Stack => plot_channels(conf, data.span, &chans, false, plot_id, size, ui),
        ValueLayout::Expand => stacked(chans.len(), size, ui, |i, sub_size, ui| {
            let chan = chans.get(i).map(std::slice::from_ref).unwrap_or(&[]);
            plot_channels(conf, data.span, chan, true, plot_id.with(i), sub_size, ui)
        }),
    }
}

/// Draw `chans` in one plot over `span` filling `size`. With `titled`, a
/// channel's key text is shown at the top-left.
fn plot_channels(
    conf: DrawConf<'_>,
    span: [f64; 2],
    chans: &[(&ChannelKey, &Channel)],
    titled: bool,
    plot_id: egui::Id,
    size: egui::Vec2,
    ui: &mut egui::Ui,
) -> egui::Response {
    let look = conf.look;
    let frame = look.frame();
    let base = resolve_color(look.color, ui);
    let colors: Vec<egui::Color32> = chans
        .iter()
        .map(|(key, _)| key_color(conf.key_colors, key).unwrap_or(base))
        .collect();

    // Place each channel's non-numeric events in lanes below those of the
    // prior channels.
    let mut boxes = vec![];
    let mut n_lanes = 0;
    for ((key, chan), &color) in chans.iter().zip(&colors) {
        let segs: Vec<_> = chan
            .segments
            .iter()
            .filter(|s| s.leaf.num().is_none())
            .collect();
        let spans: Vec<[f64; 2]> = segs.iter().map(|s| [s.start, s.end]).collect();
        let (lanes, n) = pack_lanes(&spans);
        for (seg, lane) in segs.iter().zip(lanes) {
            boxes.push(LaneBox {
                key,
                leaf: &seg.leaf,
                start: seg.start,
                end: seg.end,
                lane: n_lanes + lane,
                color,
            });
        }
        n_lanes += n;
    }

    // Fit the value axis to the numeric data. With none, the axis spans the
    // lanes. Fitted edges are padded so onset dots at the extremes draw whole.
    // Fixed value bounds stay exact.
    let values: Vec<f64> = chans
        .iter()
        .flat_map(|(_, c)| {
            let segs = c.segments.iter().filter_map(|s| s.leaf.num());
            segs.chain(c.points.iter().map(|p| p[1]))
        })
        .collect();
    let (ylo, yhi) = if values.is_empty() {
        (0.0, n_lanes.max(1) as f64)
    } else {
        let (lo, hi) = y_bounds(values.iter().copied(), false, None, None);
        let pad = edge_pad(lo, hi, size.y);
        (
            look.y_min.map_or(lo - pad, |v| f64::from(v.get())),
            look.y_max.map_or(hi + pad, |v| f64::from(v.get())),
        )
    };
    let [xlo, xhi] = span;
    let xhi = xhi.max(xlo + f64::EPSILON);
    let xpad = edge_pad(xlo, xhi, size.x);
    let bounds = ([xlo - xpad, ylo], [xhi + xpad, yhi]);
    let lane_h = (yhi - ylo) / n_lanes.max(1) as f64;
    let box_ys = |lane: usize| {
        let top = yhi - lane_h * lane as f64;
        [top - lane_h * (1.0 - LANE_GAP), top - lane_h * LANE_GAP]
    };

    let resp = show_plot(frame, plot_id, size, bounds, ui, |plot_ui| {
        for b in &boxes {
            let [y0, y1] = box_ys(b.lane);
            let pts = vec![[b.start, y0], [b.end, y0], [b.end, y1], [b.start, y1]];
            plot_ui.polygon(
                egui_plot::Polygon::new("", pts)
                    .fill_color(b.color.gamma_multiply(0.2))
                    .stroke(egui::Stroke::new(1.0, b.color.gamma_multiply(0.6)))
                    .allow_hover(false),
            );
        }
        for ((_, chan), &color) in chans.iter().zip(&colors) {
            draw_numeric(plot_ui, chan, color, frame.interactive);
        }
    });

    let transform = resp.transform;
    let clip = *transform.frame();
    let font = egui::TextStyle::Small.resolve(ui.style());
    for b in &boxes {
        if let Leaf::Label(text) = b.leaf {
            let [y0, y1] = box_ys(b.lane);
            let rect = box_rect(&transform, [b.start, b.end], [y0, y1]);
            ui.painter().with_clip_rect(rect.intersect(clip)).text(
                rect.left_center() + egui::vec2(LABEL_INSET, 0.0),
                egui::Align2::LEFT_CENTER,
                text,
                font.clone(),
                b.color,
            );
        }
    }
    if titled && let Some(title) = chans.first().and_then(|(key, _)| key.text()) {
        ui.painter().with_clip_rect(clip).text(
            clip.left_top() + egui::vec2(LABEL_INSET, LABEL_INSET),
            egui::Align2::LEFT_TOP,
            title,
            font,
            ui.visuals().weak_text_color(),
        );
    }

    // Numeric data draws over the boxes, so it takes hover first.
    let response = resp.response;
    let hovered = response.hover_pos().and_then(|pos| {
        hit_segment(&transform, chans, pos).or_else(|| {
            let b = boxes.iter().rev().find(|b| {
                let [y0, y1] = box_ys(b.lane);
                box_rect(&transform, [b.start, b.end], [y0, y1]).contains(pos)
            })?;
            Some(hover_text(b.key, b.leaf))
        })
    });
    match hovered {
        Some(text) => response.on_hover_ui_at_pointer(|ui| {
            ui.label(text);
        }),
        None => response,
    }
}

/// Draw a channel's numeric segments, onset dots and signal line.
fn draw_numeric(
    plot_ui: &mut egui_plot::PlotUi,
    chan: &Channel,
    color: egui::Color32,
    interactive: bool,
) {
    let segs = || {
        chan.segments
            .iter()
            .filter_map(|s| s.leaf.num().map(|y| (s, y)))
    };
    for (s, y) in segs() {
        plot_ui.line(
            egui_plot::Line::new("", vec![[s.start, y], [s.end, y]])
                .color(color)
                .width(SEGMENT_WIDTH)
                .allow_hover(interactive),
        );
    }
    let onsets: Vec<[f64; 2]> = segs()
        .filter(|(s, _)| s.onset)
        .map(|(s, y)| [s.start, y])
        .collect();
    if !onsets.is_empty() {
        plot_ui.points(
            egui_plot::Points::new("", onsets)
                .color(color)
                .radius(ONSET_RADIUS)
                .filled(true)
                .allow_hover(interactive),
        );
    }
    if !chan.points.is_empty() {
        plot_ui.line(
            egui_plot::Line::new("", chan.points.clone())
                .color(color)
                .allow_hover(interactive),
        );
    }
}

/// The hover text of the last-drawn numeric segment within [`HOVER_DIST`]
/// of `pos`.
fn hit_segment(
    transform: &PlotTransform,
    chans: &[(&ChannelKey, &Channel)],
    pos: egui::Pos2,
) -> Option<String> {
    chans.iter().rev().find_map(|(key, chan)| {
        chan.segments.iter().rev().find_map(|s| {
            let y = s.leaf.num()?;
            let a = transform.position_from_point(&PlotPoint::new(s.start, y));
            let b = transform.position_from_point(&PlotPoint::new(s.end, y));
            let near = (pos.y - a.y).abs() <= HOVER_DIST
                && pos.x >= a.x - HOVER_DIST
                && pos.x <= b.x + HOVER_DIST;
            near.then(|| hover_text(key, &s.leaf))
        })
    })
}

/// The screen rect of a box over `xs` and `ys` in plot units.
fn box_rect(transform: &PlotTransform, xs: [f64; 2], ys: [f64; 2]) -> egui::Rect {
    transform.rect_from_values(&PlotPoint::new(xs[0], ys[0]), &PlotPoint::new(xs[1], ys[1]))
}

/// The hover text for a value, prefixed by its key when it has one.
fn hover_text(key: &ChannelKey, leaf: &Leaf) -> String {
    match key.text() {
        Some(k) => format!("{k}: {}", leaf.text()),
        None => leaf.text(),
    }
}

/// The colour set for a channel's key in `key_colors`. The whole channel has
/// no key and so no set colour.
pub(super) fn key_color(key_colors: &[KeyColor], key: &ChannelKey) -> Option<egui::Color32> {
    let text = key.text()?;
    key_colors.iter().find(|kc| kc.key == text).map(|kc| {
        let [r, g, b, a] = kc.color;
        egui::Color32::from_rgba_unmultiplied(r, g, b, a)
    })
}

/// Greedily pack `spans` into lanes, so spans sharing a lane never overlap.
/// Spans take the first lane free at their start, in start order. Returns
/// each span's lane and the lane count.
pub(super) fn pack_lanes(spans: &[[f64; 2]]) -> (Vec<usize>, usize) {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by(|&a, &b| spans[a][0].total_cmp(&spans[b][0]));
    let mut lane_ends: Vec<f64> = vec![];
    let mut lanes = vec![0; spans.len()];
    for i in order {
        let [start, end] = spans[i];
        let lane = match lane_ends.iter().position(|&e| e <= start) {
            Some(lane) => lane,
            None => {
                lane_ends.push(end);
                lane_ends.len() - 1
            }
        };
        lane_ends[lane] = end;
        lanes[i] = lane;
    }
    (lanes, lane_ends.len())
}

/// The padding in plot units, on each side of `lo..hi` drawn over `len`
/// points, that leaves [`EDGE_PAD`] points between the data and each edge.
/// Zero when `len` leaves no room for it.
fn edge_pad(lo: f64, hi: f64, len: f32) -> f64 {
    let room = f64::from(len) - 2.0 * f64::from(EDGE_PAD);
    if room > 0.0 {
        f64::from(EDGE_PAD) * (hi - lo) / room
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Disjoint and abutting spans share a lane. Overlaps split.
    #[test]
    fn lanes_pack_greedily() {
        assert_eq!(pack_lanes(&[]), (vec![], 0));
        assert_eq!(pack_lanes(&[[0.0, 0.5], [0.5, 1.0]]), (vec![0, 0], 1),);
        // `[bd, hh hh]`: bd over the whole cycle, hh twice.
        assert_eq!(
            pack_lanes(&[[0.0, 1.0], [0.0, 0.5], [0.5, 1.0]]),
            (vec![0, 1, 1], 2),
        );
        // Start order decides, not input order.
        assert_eq!(pack_lanes(&[[0.5, 1.0], [0.0, 0.75]]), (vec![1, 0], 2),);
    }

    // A key colour applies by key text, and by decimal for an index. The
    // whole channel has none.
    #[test]
    fn key_colors_by_text() {
        let kcs = [
            KeyColor {
                key: "s".into(),
                color: [255, 0, 0, 255],
            },
            KeyColor {
                key: "1".into(),
                color: [0, 255, 0, 255],
            },
        ];
        assert_eq!(
            key_color(&kcs, &ChannelKey::Key("s".into())),
            Some(egui::Color32::RED)
        );
        assert_eq!(
            key_color(&kcs, &ChannelKey::Index(1)),
            Some(egui::Color32::from_rgb(0, 255, 0))
        );
        assert_eq!(key_color(&kcs, &ChannelKey::Index(0)), None);
        assert_eq!(key_color(&kcs, &ChannelKey::Whole), None);
    }

    // The pad leaves `EDGE_PAD` points at each edge. With no room for it,
    // there is none.
    #[test]
    fn edge_pad_fits_onset_dots() {
        let len = 100.0;
        let pad = edge_pad(0.0, 1.0, len);
        let pts_per_unit = f64::from(len) / (1.0 + 2.0 * pad);
        assert!((pad * pts_per_unit - f64::from(EDGE_PAD)).abs() < 1e-9);
        assert_eq!(edge_pad(0.0, 1.0, 2.0 * EDGE_PAD), 0.0);
    }
}
