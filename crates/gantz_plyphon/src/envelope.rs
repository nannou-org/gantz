//! Envelope breakpoints for the `~envgen` node, in the layout of plyphon's
//! `EnvGen`.
//!
//! An envelope starts at `init` and moves through its segments in order. Each
//! segment goes to its `level` over its `time` in seconds, with a [`Shape`].
//! Point 0 is the start. Point `k` is the end of segment `k - 1`. The release
//! point, if any, is where a held gate sustains. When the gate closes, the
//! envelope continues from that point.

use serde::{Deserialize, Serialize};

/// The shape of one envelope segment. The codes match plyphon's `EnvGen`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Shape {
    /// Jump to the level at the start of the segment.
    #[serde(rename = "step")]
    Step,
    /// A straight line.
    #[default]
    #[serde(rename = "lin")]
    Lin,
    /// An exponential sweep. A level of 0 is treated as a small value.
    #[serde(rename = "exp")]
    Exp,
    /// An S-shaped sine curve.
    #[serde(rename = "sin")]
    Sin,
    /// A quarter sine.
    #[serde(rename = "wel")]
    Welch,
    /// A curve that the segment's `curve` value bends. 0 is a straight line.
    /// A negative value is fast at the start. A positive value is slow at
    /// the start.
    #[serde(rename = "curve")]
    Curve,
    /// A straight line in square root space.
    #[serde(rename = "sqr")]
    Squared,
    /// A straight line in cube root space.
    #[serde(rename = "cub")]
    Cubed,
    /// Hold the start level, then jump to the level at the end.
    #[serde(rename = "hold")]
    Hold,
}

/// One envelope segment.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    /// The level at the end of the segment.
    pub level: f32,
    /// The duration in seconds.
    pub time: f32,
    /// How the level moves.
    #[serde(default, skip_serializing_if = "is_default_shape")]
    pub shape: Shape,
    /// The bend of a [`Shape::Curve`] segment.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub curve: f32,
}

/// An envelope. See the module docs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    /// The level at the start.
    pub init: f32,
    /// The segments, in order.
    pub segments: Vec<Segment>,
    /// The point where a held gate sustains, in `1..=segments.len()`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<usize>,
}

impl Shape {
    /// Every shape, in code order.
    pub const ALL: [Shape; 9] = [
        Shape::Step,
        Shape::Lin,
        Shape::Exp,
        Shape::Sin,
        Shape::Welch,
        Shape::Curve,
        Shape::Squared,
        Shape::Cubed,
        Shape::Hold,
    ];

    /// The shape code that plyphon's `EnvGen` reads.
    pub fn code(self) -> f32 {
        match self {
            Shape::Step => 0.0,
            Shape::Lin => 1.0,
            Shape::Exp => 2.0,
            Shape::Sin => 3.0,
            Shape::Welch => 4.0,
            Shape::Curve => 5.0,
            Shape::Squared => 6.0,
            Shape::Cubed => 7.0,
            Shape::Hold => 8.0,
        }
    }

    /// The short name, as in the `.gantz` sugar.
    pub fn name(self) -> &'static str {
        match self {
            Shape::Step => "step",
            Shape::Lin => "lin",
            Shape::Exp => "exp",
            Shape::Sin => "sin",
            Shape::Welch => "wel",
            Shape::Curve => "curve",
            Shape::Squared => "sqr",
            Shape::Cubed => "cub",
            Shape::Hold => "hold",
        }
    }

    /// The shape with the short `name`, if any.
    pub fn from_name(name: &str) -> Option<Shape> {
        Shape::ALL.into_iter().find(|s| s.name() == name)
    }

    /// A readable label, for the inspector.
    pub fn label(self) -> &'static str {
        match self {
            Shape::Step => "step",
            Shape::Lin => "linear",
            Shape::Exp => "exponential",
            Shape::Sin => "sine",
            Shape::Welch => "welch",
            Shape::Curve => "curve",
            Shape::Squared => "squared",
            Shape::Cubed => "cubed",
            Shape::Hold => "hold",
        }
    }
}

impl Segment {
    /// A segment to `level` over `time` seconds with `shape`.
    pub fn new(level: f32, time: f32, shape: Shape) -> Self {
        Segment {
            level,
            time,
            shape,
            curve: 0.0,
        }
    }

    /// A [`Shape::Curve`] segment to `level` over `time` seconds, bent by
    /// `curve`.
    pub fn curved(level: f32, time: f32, curve: f32) -> Self {
        Segment {
            level,
            time,
            shape: Shape::Curve,
            curve,
        }
    }
}

impl Envelope {
    /// An attack, decay, sustain and release envelope. It rises to 1, falls
    /// to `sustain` and holds there while the gate is open.
    pub fn adsr(attack: f32, decay: f32, sustain: f32, release: f32) -> Self {
        Envelope {
            init: 0.0,
            segments: vec![
                Segment::curved(1.0, attack, -4.0),
                Segment::curved(sustain, decay, -4.0),
                Segment::curved(0.0, release, -4.0),
            ],
            release: Some(2),
        }
    }

    /// A percussive envelope. It rises to 1 and falls to 0, with no sustain.
    pub fn perc(attack: f32, release: f32) -> Self {
        Envelope {
            init: 0.0,
            segments: vec![
                Segment::curved(1.0, attack, -4.0),
                Segment::curved(0.0, release, -4.0),
            ],
            release: None,
        }
    }

    /// An attack, sustain and release envelope. It rises to `sustain` and
    /// holds there while the gate is open.
    pub fn asr(attack: f32, sustain: f32, release: f32) -> Self {
        Envelope {
            init: 0.0,
            segments: vec![
                Segment::curved(sustain, attack, -4.0),
                Segment::curved(0.0, release, -4.0),
            ],
            release: Some(1),
        }
    }

    /// A straight rise to 1 and fall to 0 over `dur` seconds.
    pub fn triangle(dur: f32) -> Self {
        Envelope {
            init: 0.0,
            segments: vec![
                Segment::new(1.0, dur / 2.0, Shape::Lin),
                Segment::new(0.0, dur / 2.0, Shape::Lin),
            ],
            release: None,
        }
    }

    /// The number of points. This is one more than the number of segments.
    pub fn n_points(&self) -> usize {
        self.segments.len() + 1
    }

    /// The time and level of point `k`, if it exists.
    pub fn point(&self, k: usize) -> Option<(f32, f32)> {
        match k {
            0 => Some((0.0, self.init)),
            k => {
                let seg = self.segments.get(k - 1)?;
                let time = self.segments[..k].iter().map(|s| s.time.max(0.0)).sum();
                Some((time, seg.level))
            }
        }
    }

    /// The level at the start of segment `i`.
    pub fn start_level(&self, i: usize) -> f32 {
        match i {
            0 => self.init,
            i => self.segments.get(i - 1).map_or(self.init, |s| s.level),
        }
    }

    /// The sum of the segment times.
    pub fn total_time(&self) -> f32 {
        self.segments.iter().map(|s| s.time.max(0.0)).sum()
    }

    /// The level at `t` seconds, as `EnvGen` plays the envelope from a single
    /// gate with no release. The level holds at `init` before 0 and at the
    /// last level after the end.
    pub fn level_at(&self, t: f32) -> f32 {
        if t <= 0.0 {
            return self.init;
        }
        let mut start_t = 0.0;
        let mut start = self.init;
        for seg in &self.segments {
            let time = seg.time.max(0.0);
            if t < start_t + time {
                let frac = (t - start_t) / time;
                return shape_level(seg.shape, seg.curve, start, seg.level, frac);
            }
            start_t += time;
            start = seg.level;
        }
        start
    }

    /// Insert a point at `t` seconds and return its index. Inside the
    /// envelope, the point splits the segment at `t`, at the level of the
    /// envelope there. After the end, a new straight segment holds the last
    /// level until `t`.
    pub fn insert_point(&mut self, t: f32) -> usize {
        let t = t.max(0.0);
        let mut start_t = 0.0;
        for i in 0..self.segments.len() {
            let seg = self.segments[i];
            let time = seg.time.max(0.0);
            if t < start_t + time {
                let before = t - start_t;
                let level = self.level_at(t);
                self.segments[i].time = time - before;
                self.segments.insert(
                    i,
                    Segment {
                        level,
                        time: before,
                        ..seg
                    },
                );
                self.shift_release(i + 1, 1);
                return i + 1;
            }
            start_t += time;
        }
        let level = self.segments.last().map_or(self.init, |s| s.level);
        self.segments
            .push(Segment::new(level, t - start_t, Shape::Lin));
        self.segments.len()
    }

    /// Remove point `k` in `1..n_points()` and return whether it existed.
    /// The segments on both sides of the point join, so later points keep
    /// their time. Point 0 is the start and cannot be removed.
    pub fn remove_point(&mut self, k: usize) -> bool {
        if k == 0 || k > self.segments.len() {
            return false;
        }
        let removed = self.segments.remove(k - 1);
        if let Some(next) = self.segments.get_mut(k - 1) {
            next.time += removed.time.max(0.0);
        }
        if self.release == Some(k) {
            self.release = None;
        }
        self.shift_release(k + 1, -1);
        true
    }

    /// Move the release point by `by` if it is at or after point `from`.
    fn shift_release(&mut self, from: usize, by: isize) {
        if let Some(r) = self.release.as_mut() {
            if *r >= from {
                *r = r.saturating_add_signed(by);
            }
        }
    }
}

impl Default for Envelope {
    /// The ADSR of SuperCollider's `Env.adsr`.
    fn default() -> Self {
        Envelope::adsr(0.01, 0.3, 0.5, 1.0)
    }
}

/// The level at fraction `frac` of a segment from `start` to `end`, as
/// plyphon's `EnvGen` computes it.
pub fn shape_level(shape: Shape, curve: f32, start: f32, end: f32, frac: f32) -> f32 {
    use std::f64::consts::{FRAC_PI_2, PI};
    let (s, e, t, c) = (
        start as f64,
        end as f64,
        frac.clamp(0.0, 1.0) as f64,
        curve as f64,
    );
    let level = match shape {
        Shape::Step => e,
        Shape::Lin => s + (e - s) * t,
        Shape::Exp => {
            let s = if s.abs() < 1e-5 {
                1e-5_f64.copysign(if e == 0.0 { 1.0 } else { e })
            } else {
                s
            };
            let e = if e.abs() < 1e-5 {
                1e-5_f64.copysign(s)
            } else {
                e
            };
            s * (e / s).powf(t)
        }
        Shape::Sin => s + (e - s) * (0.5 - 0.5 * (PI * t).cos()),
        Shape::Welch => match s <= e {
            true => s + (e - s) * (FRAC_PI_2 * t).sin(),
            false => e - (e - s) * (FRAC_PI_2 - FRAC_PI_2 * t).sin(),
        },
        Shape::Curve => match c.abs() < 0.001 {
            true => s + (e - s) * t,
            false => s + (e - s) * (1.0 - (t * c).exp()) / (1.0 - c.exp()),
        },
        Shape::Squared => {
            let (y1, y2) = (s.sqrt(), e.sqrt());
            let y = y1 + (y2 - y1) * t;
            y * y
        }
        Shape::Cubed => {
            let (y1, y2) = (s.powf(1.0 / 3.0), e.powf(1.0 / 3.0));
            let y = y1 + (y2 - y1) * t;
            y * y * y
        }
        Shape::Hold => match t >= 1.0 {
            true => e,
            false => s,
        },
    };
    level as f32
}

fn is_default_shape(shape: &Shape) -> bool {
    *shape == Shape::default()
}

fn is_zero(x: &f32) -> bool {
    *x == 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn shapes_match_the_envgen_formulas() {
        let at = |shape, curve, frac| shape_level(shape, curve, 1.0, 4.0, frac);
        for shape in Shape::ALL {
            assert!(
                close(at(shape, -4.0, 1.0), 4.0),
                "{shape:?} ends at the end level"
            );
        }
        assert!(close(at(Shape::Step, 0.0, 0.0), 4.0));
        assert!(close(at(Shape::Lin, 0.0, 0.5), 2.5));
        assert!(close(at(Shape::Exp, 0.0, 0.5), 2.0));
        assert!(close(at(Shape::Sin, 0.0, 0.5), 2.5));
        assert!(close(at(Shape::Welch, 0.0, 0.5), 1.0 + 3.0 * 0.5f32.sqrt()));
        assert!(close(at(Shape::Curve, 0.0, 0.5), 2.5), "curve 0 is linear");
        let bent = 1.0 + 3.0 * (1.0 - (-2.0f32).exp()) / (1.0 - (-4.0f32).exp());
        assert!(close(at(Shape::Curve, -4.0, 0.5), bent));
        assert!(close(at(Shape::Squared, 0.0, 0.5), 2.25));
        assert!(close(at(Shape::Cubed, 0.0, 0.0), 1.0));
        assert!(close(at(Shape::Hold, 0.0, 0.99), 1.0));
    }

    #[test]
    fn shape_codes_and_names_round_trip() {
        for (code, shape) in Shape::ALL.into_iter().enumerate() {
            assert_eq!(shape.code(), code as f32);
            assert_eq!(Shape::from_name(shape.name()), Some(shape));
        }
    }

    #[test]
    fn level_at_walks_the_segments() {
        let env = Envelope::triangle(2.0);
        assert_eq!(env.level_at(-1.0), 0.0);
        assert!(close(env.level_at(0.5), 0.5));
        assert!(close(env.level_at(1.0), 1.0));
        assert!(close(env.level_at(1.5), 0.5));
        assert_eq!(env.level_at(3.0), 0.0, "holds the last level");
        assert!(close(env.total_time(), 2.0));
        assert_eq!(env.point(1), Some((1.0, 1.0)));
        assert_eq!(env.point(3), None);
    }

    #[test]
    fn a_zero_time_segment_jumps() {
        let env = Envelope {
            init: 0.0,
            segments: vec![
                Segment::new(1.0, 0.0, Shape::Lin),
                Segment::new(0.0, 1.0, Shape::Lin),
            ],
            release: None,
        };
        assert!(close(env.level_at(0.25), 0.75));
    }

    #[test]
    fn inserting_a_point_keeps_the_shape_of_the_envelope() {
        let mut env = Envelope::triangle(2.0);
        env.release = Some(2);
        let k = env.insert_point(0.5);
        assert_eq!(k, 1);
        assert_eq!(env.segments.len(), 3);
        assert_eq!(env.point(1), Some((0.5, 0.5)));
        assert_eq!(env.point(2), Some((1.0, 1.0)));
        assert!(close(env.total_time(), 2.0));
        assert_eq!(env.release, Some(3), "later points shift");
        let end = env.insert_point(3.0);
        assert_eq!(end, 4);
        assert_eq!(env.point(4), Some((3.0, 0.0)));
    }

    #[test]
    fn removing_a_point_joins_its_segments() {
        let mut env = Envelope::adsr(0.1, 0.2, 0.5, 1.0);
        assert!(!env.remove_point(0), "the start stays");
        assert!(env.remove_point(1));
        assert_eq!(env.segments.len(), 2);
        let (t, level) = env.point(1).unwrap();
        assert!(
            close(t, 0.3) && level == 0.5,
            "later points keep their time"
        );
        assert_eq!(env.release, Some(1));
        assert!(env.remove_point(1));
        assert_eq!(env.release, None, "the release point is removed");
        assert!(!env.remove_point(2));
    }
}
