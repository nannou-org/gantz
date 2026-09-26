//! An envelope editor widget for the `~envgen` node body.
//!
//! The editor draws an [`Envelope`] with the same plot as the `plot` node and
//! edits it in place:
//! - Drag a point to move it. The start point moves up and down only.
//! - Drag the handle in the middle of a segment up or down to bend it. A bent
//!   segment snaps back to a straight line near the middle, unless alt is
//!   held.
//! - Double-click the plot to add a point.
//! - Right-click a point to set the shape of its segment, make it the
//!   release point or delete it.
//!
//! Only the handles sense a drag, so the rest of the body still selects and
//! moves the node.

use crate::envelope::{Envelope, Shape, shape_level};
use egui_plot::{HLine, Line, LineStyle, PlotPoint, PlotPoints, PlotTransform, VLine};
use gantz_egui::ui_tree::plot::{PlotFrame, show_plot};

/// The editor's area and what it did to the envelope this frame.
pub struct EditorResponse {
    /// The response of the plot area.
    pub response: egui::Response,
    /// The mapping between envelope values and plot positions.
    pub transform: PlotTransform,
    /// The edits of this frame.
    pub edits: Edits,
}

/// What the editor did to the envelope this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Edits {
    /// A handle is being dragged.
    pub active: bool,
    /// A drag changed the envelope. The segment count is the same.
    pub dragged: bool,
    /// A drag ended.
    pub drag_stopped: bool,
    /// A menu or a double-click changed the envelope.
    pub edited: bool,
}

/// The time and level ranges of the envelope.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Scale {
    t_max: f32,
    lo: f32,
    hi: f32,
}

/// The mapping between envelope time and level and plot positions.
struct Map(PlotTransform);

/// The radius of a point handle.
const HANDLE_R: f32 = 3.5;

/// The size of the area that senses a handle.
const HANDLE_HIT: f32 = 12.0;

/// How much a vertical point drag of the curve handle bends the curve.
const CURVE_PER_POINT: f32 = 0.1;

/// The largest curve magnitude.
const CURVE_MAX: f32 = 20.0;

/// A bend closer to 0 than this snaps to a straight line.
const CURVE_SNAP: f32 = 0.5;

impl Scale {
    /// The ranges that fit `env`. The level range always includes 0.
    fn fit(env: &Envelope) -> Self {
        let levels = std::iter::once(env.init).chain(env.segments.iter().map(|s| s.level));
        let (lo, hi) = levels.fold((0.0f32, 0.0f32), |(lo, hi), l| (lo.min(l), hi.max(l)));
        let hi = match hi - lo < 1e-6 {
            true => lo + 1.0,
            false => hi,
        };
        Scale {
            t_max: env.total_time().max(1e-3),
            lo,
            hi,
        }
    }

    /// The plot bounds, with a margin so the handles at the edges show.
    fn bounds(&self) -> ([f64; 2], [f64; 2]) {
        let x_pad = self.t_max as f64 * 0.03;
        let y_pad = (self.hi - self.lo) as f64 * 0.08;
        (
            [-x_pad, self.lo as f64 - y_pad],
            [self.t_max as f64 + x_pad, self.hi as f64 + y_pad],
        )
    }
}

impl Map {
    fn pos(&self, t: f32, level: f32) -> egui::Pos2 {
        self.0
            .position_from_point(&PlotPoint::new(t as f64, level as f64))
    }

    /// The time and level at `pos`.
    fn unpos(&self, pos: egui::Pos2) -> (f32, f32) {
        let p = self.0.value_from_position(pos);
        (p.x as f32, p.y as f32)
    }
}

/// Draw `env` into a `size` area and apply the user's edits to it. `frame`
/// sets the grid and axes, as for the `plot` node.
pub fn envelope_editor(
    ui: &mut egui::Ui,
    id: egui::Id,
    env: &mut Envelope,
    size: egui::Vec2,
    frame: PlotFrame,
) -> EditorResponse {
    // The ranges stay fixed during a drag, so a point stays under the pointer.
    // A drag keeps them in temp memory until it ends.
    let scale_id = id.with("scale");
    let scale = ui
        .data(|d| d.get_temp::<Scale>(scale_id))
        .unwrap_or_else(|| Scale::fit(env));
    let visuals = ui.visuals().clone();
    let line = visuals.strong_text_color();
    let weak = visuals.weak_text_color();
    let active = visuals.selection.stroke.color;

    let points = curve_points(env, scale.t_max, size.x);
    let release_t = env.release.and_then(|k| env.point(k)).map(|(t, _)| t);
    let plot = show_plot(
        frame,
        id.with("plot"),
        size,
        scale.bounds(),
        ui,
        |plot_ui| {
            if scale.lo < 0.0 {
                plot_ui.hline(
                    HLine::new("", 0.0)
                        .color(weak)
                        .width(0.5)
                        .allow_hover(false),
                );
            }
            if let Some(t) = release_t {
                let release = VLine::new("", t as f64)
                    .color(weak)
                    .style(LineStyle::dashed_dense())
                    .allow_hover(false);
                plot_ui.vline(release);
            }
            let curve = Line::new("", PlotPoints::from(points))
                .color(line)
                .width(1.5)
                .allow_hover(false);
            plot_ui.line(curve);
        },
    );
    let map = Map(plot.transform);
    let painter = ui.painter_at(plot.transform.frame().expand(HANDLE_R + 2.0));
    let mut out = Edits::default();
    let mut handle_hovered = false;
    let mut handle_dragged = false;

    // The curve handles, one per segment with a length.
    let alt = ui.input(|i| i.modifiers.alt);
    for i in 0..env.segments.len() {
        let seg = env.segments[i];
        if seg.time <= 0.0 {
            continue;
        }
        let (t0, _) = env.point(i).expect("segment start");
        let mid_t = t0 + seg.time / 2.0;
        let pos = map.pos(mid_t, env.level_at(mid_t));
        let hit = egui::Rect::from_center_size(pos, egui::Vec2::splat(HANDLE_HIT));
        let resp = ui
            .interact(hit, id.with(("curve", i)), egui::Sense::drag())
            .on_hover_cursor(egui::CursorIcon::ResizeVertical)
            .on_hover_text(format!(
                "{} {:.2}. Drag up or down to bend. Hold alt to stop the snap to a line.",
                seg.shape.label(),
                seg.curve,
            ));
        let hot = resp.hovered() || resp.dragged();
        handle_hovered |= resp.hovered();
        handle_dragged |= resp.dragged() && !resp.drag_stopped();
        let half = if hot { 3.5 } else { 2.5 };
        let square = egui::Rect::from_center_size(pos, egui::Vec2::splat(half * 2.0));
        let stroke = egui::Stroke::new(1.0, if hot { active } else { weak });
        painter.rect_stroke(square, 0.0, stroke, egui::StrokeKind::Middle);
        // The bend before any snap. It lives in temp memory for the drag, so
        // a drag can leave the snap zone.
        let raw_id = id.with(("curve-raw", i));
        if resp.dragged() && resp.drag_delta().y != 0.0 {
            let start = env.start_level(i);
            // Dragging up bows the segment up, whichever way it goes.
            let sign = if seg.level >= start { 1.0 } else { -1.0 };
            let prev_raw = ui
                .data(|d| d.get_temp::<f32>(raw_id))
                .unwrap_or(match seg.shape {
                    Shape::Curve => seg.curve,
                    _ => 0.0,
                });
            let raw = (prev_raw + resp.drag_delta().y * CURVE_PER_POINT * sign)
                .clamp(-CURVE_MAX, CURVE_MAX);
            ui.data_mut(|d| d.insert_temp(raw_id, raw));
            let bent = bend(seg, raw, !alt);
            if bent != seg {
                env.segments[i] = bent;
                out.dragged = true;
            }
        }
        if resp.drag_stopped() {
            ui.data_mut(|d| d.remove::<f32>(raw_id));
            out.drag_stopped = true;
        }
    }

    // The point handles.
    let mut remove = None;
    for k in 0..env.n_points() {
        let (t, level) = env.point(k).expect("point");
        let pos = map.pos(t, level);
        let hit = egui::Rect::from_center_size(pos, egui::Vec2::splat(HANDLE_HIT));
        let cursor = match k {
            0 => egui::CursorIcon::ResizeVertical,
            _ => egui::CursorIcon::Move,
        };
        let resp = ui
            .interact(hit, id.with(("point", k)), egui::Sense::click_and_drag())
            .on_hover_cursor(cursor)
            .on_hover_text(format!("{t:.3} s, level {level:.3}"));
        let hot = resp.hovered() || resp.dragged();
        handle_hovered |= resp.hovered();
        handle_dragged |= resp.dragged() && !resp.drag_stopped();
        let radius = if hot { HANDLE_R + 1.0 } else { HANDLE_R };
        painter.circle_filled(pos, radius, if hot { active } else { line });
        if resp.dragged() {
            if let Some(pointer) = resp.interact_pointer_pos() {
                out.dragged |= move_point(env, k, &map, pointer);
            }
        }
        out.drag_stopped |= resp.drag_stopped();
        if k > 0 {
            resp.context_menu(|ui| {
                if point_menu(ui, env, k, &mut remove) {
                    out.edited = true;
                }
            });
        }
    }
    if let Some(k) = remove {
        out.edited |= env.remove_point(k);
    }

    // A double-click on the plot, off the handles, adds a point.
    let double_clicked = ui.input(|i| {
        i.pointer
            .button_double_clicked(egui::PointerButton::Primary)
    });
    if double_clicked && !handle_hovered {
        if let Some(pointer) = plot.response.hover_pos() {
            let (t, level) = map.unpos(pointer);
            let k = env.insert_point(t.max(0.0));
            if let Some(seg) = env.segments.get_mut(k - 1) {
                seg.level = level;
            }
            out.edited = true;
        }
    }

    // Keep the ranges of this frame for the rest of a drag.
    ui.data_mut(|d| match handle_dragged {
        true => {
            d.insert_temp(scale_id, scale);
        }
        false => d.remove::<Scale>(scale_id),
    });
    out.active = handle_dragged;
    EditorResponse {
        response: plot.response,
        transform: plot.transform,
        edits: out,
    }
}

/// `seg` bent by `curve`. With `snap`, a bend near 0 is a straight line.
fn bend(seg: crate::envelope::Segment, curve: f32, snap: bool) -> crate::envelope::Segment {
    match snap && curve.abs() < CURVE_SNAP {
        true => crate::envelope::Segment {
            shape: Shape::Lin,
            curve: 0.0,
            ..seg
        },
        false => crate::envelope::Segment {
            shape: Shape::Curve,
            curve,
            ..seg
        },
    }
}

/// The points of the curve of `env`, sampled about once per pixel of a plot
/// `width` points wide that shows `t_max` seconds.
fn curve_points(env: &Envelope, t_max: f32, width: f32) -> Vec<[f64; 2]> {
    let mut points = vec![[0.0, env.init as f64]];
    let mut t0 = 0.0;
    let mut start = env.init;
    for seg in &env.segments {
        let px = seg.time / t_max * width;
        let steps = (px.ceil() as usize).clamp(1, 512);
        for s in 1..=steps {
            let frac = s as f32 / steps as f32;
            let level = match seg.time > 0.0 {
                true => shape_level(seg.shape, seg.curve, start, seg.level, frac),
                false => seg.level,
            };
            points.push([(t0 + seg.time * frac) as f64, level as f64]);
        }
        t0 += seg.time;
        start = seg.level;
    }
    points
}

/// Move point `k` to `pointer`. A point keeps its time between its
/// neighbours, so the points after it keep their times. The last point can
/// move later. Returns whether the envelope changed.
fn move_point(env: &mut Envelope, k: usize, map: &Map, pointer: egui::Pos2) -> bool {
    let before = env.clone();
    let (t, level) = map.unpos(pointer);
    if k == 0 {
        env.init = level;
        return *env != before;
    }
    let (prev_t, _) = env.point(k - 1).expect("previous point");
    let next_t = env.point(k + 1).map(|(t, _)| t);
    let t = match next_t {
        Some(next_t) => t.clamp(prev_t, next_t),
        None => t.max(prev_t),
    };
    env.segments[k - 1].level = level;
    env.segments[k - 1].time = t - prev_t;
    if let (Some(next_t), Some(next)) = (next_t, env.segments.get_mut(k)) {
        next.time = next_t - t;
    }
    *env != before
}

/// The right-click menu of point `k`. Returns whether it changed `env`. A
/// delete is left in `remove` for after the handle loop.
fn point_menu(ui: &mut egui::Ui, env: &mut Envelope, k: usize, remove: &mut Option<usize>) -> bool {
    let mut changed = false;
    let seg = &mut env.segments[k - 1];
    ui.menu_button("Shape", |ui| {
        for shape in Shape::ALL {
            if ui.radio(seg.shape == shape, shape.label()).clicked() {
                seg.shape = shape;
                changed = true;
                ui.close();
            }
        }
    });
    let mut is_release = env.release == Some(k);
    if ui.checkbox(&mut is_release, "Release point").clicked() {
        env.release = is_release.then_some(k);
        changed = true;
        ui.close();
    }
    let can_remove = env.segments.len() > 1;
    if ui
        .add_enabled(can_remove, egui::Button::new("Delete point"))
        .clicked()
    {
        *remove = Some(k);
        ui.close();
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::Segment;
    use egui_plot::PlotBounds;

    fn map() -> Map {
        let frame = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 50.0));
        let bounds = PlotBounds::from_min_max([0.0, 0.0], [2.0, 1.0]);
        Map(PlotTransform::new(frame, bounds, false))
    }

    #[test]
    fn the_scale_fits_the_envelope_and_zero() {
        let scale = Scale::fit(&Envelope::triangle(2.0));
        assert_eq!(
            scale,
            Scale {
                t_max: 2.0,
                lo: 0.0,
                hi: 1.0
            }
        );
        let mut hz = Envelope::perc(0.01, 0.1);
        hz.init = 50.0;
        hz.segments[0].level = 230.0;
        hz.segments[1].level = 50.0;
        let scale = Scale::fit(&hz);
        assert_eq!((scale.lo, scale.hi), (0.0, 230.0));
        let ([x0, y0], [x1, y1]) = scale.bounds();
        assert!(x0 < 0.0 && y0 < 0.0 && x1 > scale.t_max as f64 && y1 > 230.0);
    }

    #[test]
    fn the_map_round_trips() {
        let map = map();
        let (t, level) = map.unpos(map.pos(0.5, 0.25));
        assert!((t - 0.5).abs() < 1e-5 && (level - 0.25).abs() < 1e-5);
    }

    #[test]
    fn a_moved_point_stays_between_its_neighbours() {
        let map = map();
        let mut env = Envelope::triangle(2.0);
        assert!(move_point(&mut env, 1, &map, map.pos(1.5, 0.5)));
        assert_eq!(env.point(1), Some((1.5, 0.5)));
        assert_eq!(
            env.point(2),
            Some((2.0, 0.0)),
            "the next point keeps its time"
        );
        move_point(&mut env, 1, &map, map.pos(5.0, 0.5));
        assert_eq!(
            env.point(1).map(|p| p.0),
            Some(2.0),
            "clamped to the next point"
        );
        move_point(&mut env, 2, &map, map.pos(3.0, 0.0));
        assert_eq!(
            env.point(2).map(|p| p.0),
            Some(3.0),
            "the last point moves later"
        );
        move_point(&mut env, 0, &map, map.pos(1.0, 0.5));
        assert_eq!(
            env.point(0),
            Some((0.0, 0.5)),
            "the start moves up and down only"
        );
    }

    #[test]
    fn a_small_bend_snaps_to_a_line() {
        let seg = Segment::new(1.0, 0.5, Shape::Lin);
        assert_eq!(bend(seg, 0.3, true), seg, "snapped");
        assert_eq!(bend(seg, 0.3, false), Segment::curved(1.0, 0.5, 0.3), "alt");
        assert_eq!(bend(seg, -2.0, true), Segment::curved(1.0, 0.5, -2.0));
        let curved = Segment::curved(1.0, 0.5, 4.0);
        assert_eq!(bend(curved, 0.1, true), seg, "a curve snaps back to a line");
    }

    #[test]
    fn the_curve_has_a_point_per_pixel() {
        let points = curve_points(&Envelope::triangle(2.0), 2.0, 100.0);
        assert_eq!(points.first(), Some(&[0.0, 0.0]));
        assert_eq!(points.last(), Some(&[2.0, 0.0]));
        assert!(points.len() > 90);
    }

    /// The editor area of the input tests.
    fn test_rect() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(200.0, 100.0))
    }

    /// Run one frame of the editor over `env` with `events` at `time`.
    fn frame(
        ctx: &egui::Context,
        env: &mut Envelope,
        time: f64,
        events: Vec<egui::Event>,
    ) -> (Edits, Map) {
        let input = egui::RawInput {
            events,
            time: Some(time),
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 300.0),
            )),
            ..Default::default()
        };
        let mut out = None;
        let _ = ctx.run_ui(input, |ui| {
            let rect = test_rect();
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                let frame = PlotFrame {
                    grid: false,
                    axes: false,
                    interactive: false,
                };
                let r = envelope_editor(ui, egui::Id::new("env"), env, rect.size(), frame);
                out = Some((r.edits, Map(r.transform)));
            });
        });
        out.expect("the editor ran")
    }

    fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// A drag of a point edits a copy live and ends once, with the point
    /// under the pointer.
    #[test]
    fn dragging_a_point_moves_it() {
        let ctx = egui::Context::default();
        let mut env = Envelope::triangle(2.0);
        let (_, map) = frame(&ctx, &mut env, 0.0, vec![]);
        let from = map.pos(1.0, 1.0);
        let to = map.pos(1.5, 0.5);
        let mid = from + (to - from) / 2.0;
        let steps = [
            vec![egui::Event::PointerMoved(from)],
            vec![press(from, true)],
            vec![egui::Event::PointerMoved(mid)],
            vec![egui::Event::PointerMoved(to)],
            vec![press(to, false)],
        ];
        let edits: Vec<Edits> = steps
            .into_iter()
            .enumerate()
            .map(|(i, events)| frame(&ctx, &mut env, 0.1 * (i + 1) as f64, events).0)
            .collect();
        assert!(edits.iter().any(|e| e.active && e.dragged), "{edits:?}");
        assert_eq!(
            edits.iter().filter(|e| e.drag_stopped).count(),
            1,
            "{edits:?}"
        );
        assert!(!edits.last().unwrap().active);
        assert!(edits.iter().all(|e| !e.edited));
        let (t, level) = env.point(1).expect("point");
        assert!(
            (t - 1.5).abs() < 0.02 && (level - 0.5).abs() < 0.02,
            "{t} {level}"
        );
        assert_eq!(env.segments.len(), 2, "a drag keeps the segments");
    }

    /// A drag of a line's curve handle bends it, and a drag back snaps it to
    /// a line.
    #[test]
    fn dragging_a_curve_handle_bends_and_snaps() {
        let ctx = egui::Context::default();
        let mut env = Envelope::triangle(2.0);
        let (_, map) = frame(&ctx, &mut env, 0.0, vec![]);
        let from = map.pos(0.5, 0.5);
        let up = from - egui::vec2(0.0, 30.0);
        let steps = [
            vec![egui::Event::PointerMoved(from)],
            vec![press(from, true)],
            vec![egui::Event::PointerMoved(up)],
        ];
        for (i, events) in steps.into_iter().enumerate() {
            frame(&ctx, &mut env, 0.1 * (i + 1) as f64, events);
        }
        assert_eq!(env.segments[0].shape, Shape::Curve, "{env:?}");
        assert!(
            env.segments[0].curve < -1.0,
            "a rising line bows up: {env:?}"
        );
        let steps = [
            vec![egui::Event::PointerMoved(from)],
            vec![press(from, false)],
        ];
        for (i, events) in steps.into_iter().enumerate() {
            frame(&ctx, &mut env, 1.0 + 0.1 * i as f64, events);
        }
        assert_eq!(
            env.segments[0],
            Segment::new(1.0, 1.0, Shape::Lin),
            "snapped back"
        );
    }

    /// A double-click on the plot adds a point under the pointer.
    #[test]
    fn double_clicking_adds_a_point() {
        let ctx = egui::Context::default();
        let mut env = Envelope::triangle(2.0);
        let (_, map) = frame(&ctx, &mut env, 0.0, vec![]);
        let at = map.pos(0.5, 0.9);
        let mut edited = false;
        let events = [
            (0.05, egui::Event::PointerMoved(at)),
            (0.1, press(at, true)),
            (0.15, press(at, false)),
            (0.2, press(at, true)),
            (0.25, press(at, false)),
            (0.3, egui::Event::PointerMoved(at)),
        ];
        for (time, event) in events {
            edited |= frame(&ctx, &mut env, time, vec![event]).0.edited;
        }
        assert!(edited);
        assert_eq!(env.segments.len(), 3);
        let (t, level) = env.point(1).expect("new point");
        assert!(
            (t - 0.5).abs() < 0.02 && (level - 0.9).abs() < 0.02,
            "{t} {level}"
        );
    }
}
