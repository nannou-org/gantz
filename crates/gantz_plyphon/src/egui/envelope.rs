//! An envelope editor widget for the `~envgen` node body.
//!
//! The editor draws an [`Envelope`] as a plot and edits it in place:
//! - Drag a point to move it. The start point moves up and down only.
//! - Drag the handle in the middle of a segment up or down to bend it.
//! - Double-click the plot to add a point.
//! - Right-click a point to set the shape of its segment, make it the
//!   release point or delete it.
//!
//! Only the handles sense a drag, so the rest of the body still selects and
//! moves the node.

use crate::envelope::{Envelope, Shape, shape_level};

/// The editor's area and what it did to the envelope this frame.
pub struct EditorResponse {
    /// The area of the editor.
    pub response: egui::Response,
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

/// The time and level ranges that map the envelope onto the plot.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Scale {
    t_max: f32,
    lo: f32,
    hi: f32,
}

/// The radius of a point handle.
const HANDLE_R: f32 = 3.5;

/// The size of the area that senses a handle.
const HANDLE_HIT: f32 = 12.0;

/// How much a vertical point drag of the curve handle bends the curve.
const CURVE_PER_POINT: f32 = 0.1;

/// The largest curve magnitude.
const CURVE_MAX: f32 = 20.0;

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
}

/// Draw `env` into a `size` area and apply the user's edits to it.
pub fn envelope_editor(
    ui: &mut egui::Ui,
    id: egui::Id,
    env: &mut Envelope,
    size: egui::Vec2,
) -> EditorResponse {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    let plot = rect.shrink(HANDLE_R + 2.0);
    // The ranges stay fixed during a drag, so a point stays under the pointer.
    // A drag keeps them in temp memory until it ends.
    let scale_id = id.with("scale");
    let frozen = ui.data(|d| d.get_temp::<Scale>(scale_id));
    let dragging = frozen.is_some();
    let scale = frozen.unwrap_or_else(|| Scale::fit(env));
    let map = Map { plot, scale };
    let hovered = ui.rect_contains_pointer(rect);
    let mut handle_hovered = false;
    let mut handle_dragged = false;

    let mut out = Edits::default();
    let visuals = ui.visuals().clone();
    let line = visuals.strong_text_color();
    let weak = visuals.weak_text_color();
    let active = visuals.selection.stroke.color;
    let painter = ui.painter_at(rect);

    // The zero line and the release point.
    if scale.lo < 0.0 {
        let y = map.y(0.0);
        let stroke = egui::Stroke::new(0.5, weak);
        painter.line_segment(
            [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
            stroke,
        );
    }
    if let Some((t, _)) = env.release.and_then(|k| env.point(k)) {
        let x = map.x(t);
        let top_bottom = [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())];
        let stroke = egui::Stroke::new(1.0, weak);
        painter.extend(egui::Shape::dashed_line(&top_bottom, stroke, 3.0, 3.0));
    }
    painter.add(egui::Shape::line(
        curve_points(env, &map),
        egui::Stroke::new(1.5, line),
    ));

    // The curve handles, one per segment that bends.
    let show_curves = hovered || dragging;
    for i in 0..env.segments.len() {
        let seg = env.segments[i];
        let bends = matches!(seg.shape, Shape::Lin | Shape::Curve) && seg.time > 0.0;
        if !(show_curves && bends) {
            continue;
        }
        let (t0, _) = env.point(i).expect("segment start");
        let mid_t = t0 + seg.time / 2.0;
        let pos = map.pos(mid_t, env.level_at(mid_t));
        let hit = egui::Rect::from_center_size(pos, egui::Vec2::splat(HANDLE_HIT));
        let resp = ui
            .interact(hit, id.with(("curve", i)), egui::Sense::drag())
            .on_hover_cursor(egui::CursorIcon::ResizeVertical)
            .on_hover_text(format!("curve {:.2}", seg.curve));
        let hot = resp.hovered() || resp.dragged();
        handle_hovered |= resp.hovered();
        handle_dragged |= resp.dragged() && !resp.drag_stopped();
        let half = if hot { 3.5 } else { 2.5 };
        let square = egui::Rect::from_center_size(pos, egui::Vec2::splat(half * 2.0));
        let stroke = egui::Stroke::new(1.0, if hot { active } else { weak });
        painter.rect_stroke(square, 0.0, stroke, egui::StrokeKind::Middle);
        if resp.dragged() && resp.drag_delta().y != 0.0 {
            let start = env.start_level(i);
            // Dragging up bows the segment up, whichever way it goes.
            let sign = if seg.level >= start { 1.0 } else { -1.0 };
            let curve = seg.curve + resp.drag_delta().y * CURVE_PER_POINT * sign;
            let seg = &mut env.segments[i];
            seg.shape = Shape::Curve;
            seg.curve = curve.clamp(-CURVE_MAX, CURVE_MAX);
            out.dragged = true;
        }
        out.drag_stopped |= resp.drag_stopped();
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
    if double_clicked && hovered && !handle_hovered {
        if let Some(pointer) = ui.input(|i| i.pointer.hover_pos()) {
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
        response,
        edits: out,
    }
}

/// The mapping between envelope time and level and plot positions.
struct Map {
    plot: egui::Rect,
    scale: Scale,
}

impl Map {
    fn x(&self, t: f32) -> f32 {
        egui::lerp(self.plot.x_range(), t / self.scale.t_max)
    }

    fn y(&self, level: f32) -> f32 {
        let frac = (level - self.scale.lo) / (self.scale.hi - self.scale.lo);
        egui::lerp(self.plot.bottom()..=self.plot.top(), frac)
    }

    fn pos(&self, t: f32, level: f32) -> egui::Pos2 {
        egui::pos2(self.x(t), self.y(level))
    }

    /// The time and level at `pos`.
    fn unpos(&self, pos: egui::Pos2) -> (f32, f32) {
        let tx = egui::remap(pos.x, self.plot.x_range(), 0.0..=self.scale.t_max);
        let ly = egui::remap(
            pos.y,
            self.plot.bottom()..=self.plot.top(),
            self.scale.lo..=self.scale.hi,
        );
        (tx, ly)
    }
}

/// The polyline of `env` on the plot, sampled about once per pixel.
fn curve_points(env: &Envelope, map: &Map) -> Vec<egui::Pos2> {
    let mut points = vec![map.pos(0.0, env.init)];
    let mut t0 = 0.0;
    let mut start = env.init;
    for seg in &env.segments {
        let width = (map.x(t0 + seg.time) - map.x(t0)).abs();
        let steps = (width.ceil() as usize).clamp(1, 512);
        for s in 1..=steps {
            let frac = s as f32 / steps as f32;
            let level = match seg.time > 0.0 {
                true => shape_level(seg.shape, seg.curve, start, seg.level, frac),
                false => seg.level,
            };
            points.push(map.pos(t0 + seg.time * frac, level));
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

    fn map() -> Map {
        let plot = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 50.0));
        let scale = Scale::fit(&Envelope::triangle(2.0));
        Map { plot, scale }
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
    }

    #[test]
    fn the_map_round_trips() {
        let map = map();
        let pos = map.pos(0.5, 0.25);
        let (t, level) = map.unpos(pos);
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
    fn the_curve_has_a_point_per_pixel() {
        let map = map();
        let points = curve_points(&Envelope::triangle(2.0), &map);
        assert_eq!(points.first(), Some(&map.pos(0.0, 0.0)));
        assert_eq!(points.last(), Some(&map.pos(2.0, 0.0)));
        assert!(points.len() > 90);
    }

    /// The editor area of the input tests.
    fn test_rect() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(200.0, 100.0))
    }

    /// The plot map of the input tests, for `env` before any edit.
    fn test_map(env: &Envelope) -> Map {
        let plot = test_rect().shrink(HANDLE_R + 2.0);
        Map {
            plot,
            scale: Scale::fit(env),
        }
    }

    /// Run one frame of the editor over `env` with `events` at `time`.
    fn frame(
        ctx: &egui::Context,
        env: &mut Envelope,
        time: f64,
        events: Vec<egui::Event>,
    ) -> Edits {
        let input = egui::RawInput {
            events,
            time: Some(time),
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 300.0),
            )),
            ..Default::default()
        };
        let mut edits = Edits::default();
        let _ = ctx.run_ui(input, |ui| {
            let rect = test_rect();
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                edits = envelope_editor(ui, egui::Id::new("env"), env, rect.size()).edits;
            });
        });
        edits
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
        let map = test_map(&env);
        let from = map.pos(1.0, 1.0);
        let to = map.pos(1.5, 0.5);
        let mid = from + (to - from) / 2.0;
        let mut edits = vec![];
        edits.push(frame(
            &ctx,
            &mut env,
            0.0,
            vec![egui::Event::PointerMoved(from)],
        ));
        edits.push(frame(&ctx, &mut env, 0.1, vec![press(from, true)]));
        edits.push(frame(
            &ctx,
            &mut env,
            0.2,
            vec![egui::Event::PointerMoved(mid)],
        ));
        edits.push(frame(
            &ctx,
            &mut env,
            0.3,
            vec![egui::Event::PointerMoved(to)],
        ));
        edits.push(frame(&ctx, &mut env, 0.4, vec![press(to, false)]));
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

    /// A double-click on the plot adds a point under the pointer.
    #[test]
    fn double_clicking_adds_a_point() {
        let ctx = egui::Context::default();
        let mut env = Envelope::triangle(2.0);
        let at = test_map(&env).pos(0.5, 0.9);
        let mut edited = false;
        let events = [
            (0.0, egui::Event::PointerMoved(at)),
            (0.05, press(at, true)),
            (0.1, press(at, false)),
            (0.15, press(at, true)),
            (0.2, press(at, false)),
            (0.25, egui::Event::PointerMoved(at)),
        ];
        for (time, event) in events {
            edited |= frame(&ctx, &mut env, time, vec![event]).edited;
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
