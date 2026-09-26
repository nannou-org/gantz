//! The `~envgen` envelope generator node.

use gantz_core::node::{ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_nodetag::NodeTag;
use plyphon::synthdef::{InputRef, UnitSpec};
use serde::{Deserialize, Serialize};

use crate::dsp::{DspBuilder, NodeDsp, NodeRate, Signal, ToNodeDsp};
use crate::envelope::Envelope;
use crate::node::is_default;
use crate::node::unit::channel_select;
use crate::param::{control_inputs_expr, param_name, plyphon_param, sync_params_state};

/// An envelope generator. It plays its [`Envelope`] with plyphon's `EnvGen`.
///
/// A rising gate starts the envelope from its current level. A held gate
/// sustains at the release point, if there is one. A falling gate then
/// continues from the release point. The output is
/// `level * scale + bias`, and `tscale` scales every segment time.
///
/// The envelope is node data, so it is saved and an edit is an undo step.
/// Each segment value is also a synth param. An edit to a level, time,
/// shape or curve changes the running synth, and it takes effect when the
/// segment next starts. An edit to the number of segments, the release point
/// or the rate derives a new synthdef.
///
/// Each of the four inputs takes a dsp signal, or a number from the control
/// side. The gate is read once per block, so a trigger into the gate must
/// last at least one block, for example a `kr` `~impulse`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, NodeTag)]
pub struct Envgen {
    #[serde(flatten)]
    env: Envelope,
    #[serde(default, skip_serializing_if = "is_default")]
    rate: NodeRate,
    #[serde(default = "default_width", skip_serializing_if = "is_default_width")]
    width: u16,
    #[serde(default = "default_height", skip_serializing_if = "is_default_height")]
    height: u16,
    #[serde(default, skip_serializing_if = "is_default")]
    grid: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    axes: bool,
    #[serde(default = "default_range", skip_serializing_if = "is_default_range")]
    x_range: [f32; 2],
    #[serde(default = "default_range", skip_serializing_if = "is_default_range")]
    y_range: [f32; 2],
    #[serde(default, skip_serializing_if = "is_default")]
    compact: bool,
}

/// A socket of the `~envgen` node, a hybrid param.
#[derive(Clone, Copy, Debug)]
pub struct Socket {
    /// The param name and inspector label.
    pub name: &'static str,
    /// The value when nothing drives the socket.
    pub default: f32,
    /// The socket doc line.
    pub doc: &'static str,
}

/// How a socket feeds `EnvGen`.
enum Feed {
    /// A connected socket's signal.
    Wire(Signal),
    /// An unconnected socket's param.
    Param(InputRef),
}

/// The sockets of the `~envgen` node, in input order. They are the first four
/// inputs of `EnvGen`.
pub const SOCKETS: [Socket; 4] = [
    Socket {
        name: "gate",
        default: 1.0,
        doc: "a rise above 0 starts the envelope from its current level. A fall \
              to 0 or below lets a waiting envelope go on from its release point. \
              Unconnected, it is 1, so the envelope plays once as the synth starts",
    },
    Socket {
        name: "scale",
        default: 1.0,
        doc: "multiplies the level",
    },
    Socket {
        name: "bias",
        default: 0.0,
        doc: "adds to the scaled level",
    },
    Socket {
        name: "tscale",
        default: 1.0,
        doc: "multiplies every segment time",
    },
];

/// The `EnvGen` release node value for no release point.
const NO_NODE: f32 = -99.0;

impl Envgen {
    /// The body width a fresh `~envgen` starts with.
    pub const DEFAULT_WIDTH: u16 = 200;

    /// The body height a fresh `~envgen` starts with.
    pub const DEFAULT_HEIGHT: u16 = 80;

    /// A node that plays `env`.
    pub fn new(env: Envelope) -> Self {
        Envgen {
            env: sanitize(env),
            rate: NodeRate::default(),
            width: Self::DEFAULT_WIDTH,
            height: Self::DEFAULT_HEIGHT,
            grid: false,
            axes: false,
            x_range: default_range(),
            y_range: default_range(),
            compact: false,
        }
    }

    /// The same node, at `rate`.
    pub fn with_rate(self, rate: NodeRate) -> Self {
        Envgen { rate, ..self }
    }

    /// The envelope.
    pub fn envelope(&self) -> &Envelope {
        &self.env
    }

    /// Set the envelope. Negative times become 0, and a release point that
    /// is not a point of the envelope is removed.
    pub fn set_envelope(&mut self, env: Envelope) {
        self.env = sanitize(env);
    }

    /// The ugen rate.
    pub fn rate(&self) -> NodeRate {
        self.rate
    }

    /// Set the ugen rate. It is structural.
    pub fn set_rate(&mut self, rate: NodeRate) {
        self.rate = rate;
    }

    /// The body size in points.
    pub fn size(&self) -> [u16; 2] {
        [self.width, self.height]
    }

    /// Set the body size in points.
    pub fn set_size(&mut self, [width, height]: [u16; 2]) {
        self.width = width;
        self.height = height;
    }

    /// Whether the body plot draws a grid.
    pub fn grid(&self) -> bool {
        self.grid
    }

    /// Set whether the body plot draws a grid.
    pub fn set_grid(&mut self, grid: bool) {
        self.grid = grid;
    }

    /// Whether the body plot draws axes with time and level labels.
    pub fn axes(&self) -> bool {
        self.axes
    }

    /// Set whether the body plot draws axes.
    pub fn set_axes(&mut self, axes: bool) {
        self.axes = axes;
    }

    /// The time range of the plot in seconds, `[min, max]`.
    pub fn x_range(&self) -> [f32; 2] {
        self.x_range
    }

    /// Set the time range of the plot. See [`range`].
    pub fn set_x_range(&mut self, x_range: [f32; 2]) {
        self.x_range = range(x_range);
    }

    /// The level range of the plot, `[min, max]`.
    pub fn y_range(&self) -> [f32; 2] {
        self.y_range
    }

    /// Set the level range of the plot. See [`range`].
    pub fn set_y_range(&mut self, y_range: [f32; 2]) {
        self.y_range = range(y_range);
    }

    /// Whether the graph shows only the node name, with no editor.
    pub fn compact(&self) -> bool {
        self.compact
    }

    /// Set whether the graph shows only the node name.
    pub fn set_compact(&mut self, compact: bool) {
        self.compact = compact;
    }
}

impl Default for Envgen {
    fn default() -> Self {
        Envgen::new(Envelope::default())
    }
}

impl gantz_core::Node for Envgen {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        SOCKETS.len()
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        true
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        // The envelope params follow the node data. `register` runs again
        // after each structural edit, so undo and load reach the running
        // synth through the driver's param drain.
        let keep: Vec<(&str, f64)> = SOCKETS.iter().map(|s| (s.name, s.default as f64)).collect();
        let set = envelope_params(&self.env);
        let path = ctx.path();
        let vm = ctx.vm();
        let prev = gantz_core::node::state::extract_value(vm, path)
            .ok()
            .flatten();
        let state = sync_params_state(prev.as_ref(), &keep, &set);
        gantz_core::node::state::update_value(vm, path, state).unwrap()
    }

    fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult {
        let sockets: Vec<(usize, &str)> = SOCKETS
            .iter()
            .enumerate()
            .map(|(ix, s)| (ix, s.name))
            .collect();
        control_inputs_expr(&ctx, &sockets, "'()")
    }
}

impl NodeDsp for Envgen {
    fn n_dsp_inputs(&self) -> usize {
        SOCKETS.len()
    }

    fn n_dsp_outputs(&self) -> usize {
        1
    }

    fn ugens(&self, path: &[usize], inputs: &[Option<Signal>], b: &mut DspBuilder) -> Vec<Signal> {
        let sockets: Vec<Feed> = SOCKETS
            .iter()
            .enumerate()
            .map(|(ix, s)| match inputs.get(ix).cloned().flatten() {
                Some(signal) => Feed::Wire(signal),
                None => Feed::Param(keyed_param(b, path, s.name, s.default)),
            })
            .collect();
        let n = self.env.segments.len();
        let release = self.env.release.map_or(NO_NODE, |r| r as f32);
        let mut env = vec![
            InputRef::Constant(0.0),
            keyed_param(b, path, "init", self.env.init),
            InputRef::Constant(n as f32),
            InputRef::Constant(release),
            InputRef::Constant(NO_NODE),
        ];
        for (i, seg) in self.env.segments.iter().enumerate() {
            env.push(keyed_param(b, path, &level_key(i), seg.level));
            env.push(keyed_param(b, path, &time_key(i), seg.time));
            env.push(keyed_param(b, path, &shape_key(i), seg.shape.code()));
            env.push(keyed_param(b, path, &curve_key(i), seg.curve));
        }
        // One unit per channel of the widest connected socket.
        let width = sockets
            .iter()
            .filter_map(|s| match s {
                Feed::Wire(signal) => Some(signal.width()),
                Feed::Param(_) => None,
            })
            .max()
            .unwrap_or(1);
        let outputs: Signal = (0..width)
            .map(|c| {
                let ins = sockets
                    .iter()
                    .map(|s| match s {
                        Feed::Wire(signal) => channel_select(signal, c),
                        Feed::Param(param) => param.clone(),
                    })
                    .chain(env.iter().cloned())
                    .collect();
                let spec = UnitSpec::new("EnvGen", self.rate.to_plyphon(), ins, 1);
                InputRef::Unit {
                    unit: b.push_unit(spec),
                    output: 0,
                }
            })
            .collect();
        vec![outputs]
    }
}

impl ToNodeDsp for Envgen {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        Some(self)
    }
}

/// The name and value of every envelope param of `env`, the start level and
/// four params per segment.
pub fn envelope_params(env: &Envelope) -> Vec<(String, f64)> {
    let segments = env.segments.iter().enumerate().flat_map(|(i, seg)| {
        [
            (level_key(i), seg.level as f64),
            (time_key(i), seg.time as f64),
            (shape_key(i), seg.shape.code() as f64),
            (curve_key(i), seg.curve as f64),
        ]
    });
    std::iter::once(("init".to_string(), env.init as f64))
        .chain(segments)
        .collect()
}

/// The param name of the level of segment `i`.
pub fn level_key(i: usize) -> String {
    format!("level-{i}")
}

/// The param name of the time of segment `i`.
pub fn time_key(i: usize) -> String {
    format!("time-{i}")
}

/// The param name of the shape code of segment `i`.
pub fn shape_key(i: usize) -> String {
    format!("shape-{i}")
}

/// The param name of the curve of segment `i`.
pub fn curve_key(i: usize) -> String {
    format!("curve-{i}")
}

/// Push the no-lag keyed param `name` with `default` and return its input.
fn keyed_param(b: &mut DspBuilder, path: &[usize], name: &str, default: f32) -> InputRef {
    let param = plyphon_param(param_name(path, name), default, 0.0);
    InputRef::Param(b.push_param_keyed(path, name, param))
}

/// `env` with no negative times and a release point that exists.
fn sanitize(mut env: Envelope) -> Envelope {
    for seg in &mut env.segments {
        seg.time = seg.time.max(0.0);
    }
    let n = env.segments.len();
    env.release = env.release.filter(|&r| (1..=n).contains(&r));
    env
}

fn default_width() -> u16 {
    Envgen::DEFAULT_WIDTH
}

fn is_default_width(width: &u16) -> bool {
    *width == Envgen::DEFAULT_WIDTH
}

fn default_height() -> u16 {
    Envgen::DEFAULT_HEIGHT
}

fn is_default_height(height: &u16) -> bool {
    *height == Envgen::DEFAULT_HEIGHT
}

/// `[min, max]` with a max above the min. An empty or reversed range keeps
/// its min and spans at least 0.001.
pub fn range([min, max]: [f32; 2]) -> [f32; 2] {
    [min, max.max(min + 0.001)]
}

fn default_range() -> [f32; 2] {
    [0.0, 1.0]
}

fn is_default_range(range: &[f32; 2]) -> bool {
    *range == default_range()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::{Segment, Shape};

    /// The units `node` emits for `inputs`, and the width of its output.
    fn emit(node: &Envgen, inputs: &[Option<Signal>]) -> (Vec<UnitSpec>, usize) {
        let mut b = DspBuilder::new(1);
        let outs = node.ugens(&[0], inputs, &mut b);
        (b.finish("t").def.units, outs[0].width())
    }

    #[test]
    fn envgen_emits_the_envgen_layout() {
        let (units, width) = emit(&Envgen::default(), &[]);
        assert_eq!(width, 1);
        let env = units.iter().find(|u| u.name == "EnvGen").expect("EnvGen");
        assert_eq!(env.inputs.len(), 9 + 4 * 3);
        let constant = |ix: usize| match env.inputs[ix] {
            InputRef::Constant(v) => v,
            ref other => panic!("input {ix} is {other:?}"),
        };
        assert_eq!(constant(4), 0.0, "no done action");
        assert_eq!(constant(6), 3.0, "three segments");
        assert_eq!(constant(7), 2.0, "the release point");
        assert_eq!(constant(8), NO_NODE, "no loop");
        for ix in [0, 1, 2, 3, 5, 9, 10, 11, 12] {
            assert!(
                matches!(env.inputs[ix], InputRef::Param(_)),
                "input {ix} is a param"
            );
        }
    }

    #[test]
    fn a_connected_gate_replaces_its_param_and_expands() {
        let gate = Signal::from_iter([InputRef::Constant(1.0), InputRef::Constant(0.0)]);
        let (units, width) = emit(&Envgen::default(), &[Some(gate)]);
        assert_eq!(width, 2, "one unit per gate channel");
        let envs: Vec<_> = units.iter().filter(|u| u.name == "EnvGen").collect();
        assert_eq!(envs.len(), 2);
        assert!(matches!(envs[1].inputs[0], InputRef::Constant(v) if v == 0.0));
        assert!(matches!(envs[1].inputs[1], InputRef::Param(_)));
    }

    #[test]
    fn values_do_not_change_the_structure() {
        let sig = |node: &Envgen| {
            let mut b = DspBuilder::new(1);
            node.ugens(&[0], &[], &mut b);
            crate::compile::structural_sig(&b.finish("t").def)
        };
        let a = Envgen::new(Envelope::perc(0.01, 1.0));
        let mut env = Envelope::perc(0.2, 0.5);
        env.segments[1].shape = Shape::Exp;
        env.init = 0.3;
        let b = Envgen::new(env);
        assert_eq!(sig(&a), sig(&b), "value edits keep the synth");
        let c = Envgen::new(Envelope::adsr(0.01, 0.3, 0.5, 1.0));
        assert_ne!(sig(&a), sig(&c), "a new segment count derives a new def");
    }

    #[test]
    fn the_envelope_is_sanitized() {
        let env = Envelope {
            init: 0.0,
            segments: vec![Segment::new(1.0, -1.0, Shape::Lin)],
            release: Some(4),
        };
        let node = Envgen::new(env);
        assert_eq!(node.envelope().segments[0].time, 0.0);
        assert_eq!(node.envelope().release, None);
    }

    #[test]
    fn envelope_params_name_every_value() {
        let params = envelope_params(&Envelope::perc(0.01, 1.0));
        let names: Vec<&str> = params.iter().map(|(n, _)| n.as_str()).collect();
        let expected = [
            "init", "level-0", "time-0", "shape-0", "curve-0", "level-1", "time-1", "shape-1",
            "curve-1",
        ];
        assert_eq!(names, expected);
    }

    #[test]
    fn default_serializes_without_rate_or_size() {
        let bare = ron::to_string(&Envgen::default()).unwrap();
        assert!(!bare.contains("rate") && !bare.contains("width"), "{bare}");
    }

    /// The sample rate of the engine tests.
    const SR: f32 = 48_000.0;

    /// Render `frames` of `node` into a mono `Out`. `gate` builds the gate
    /// wire, if any.
    fn render(
        node: &Envgen,
        gate: impl FnOnce(&mut DspBuilder) -> Option<Signal>,
        frames: usize,
    ) -> Vec<f32> {
        use plyphon::{AddAction, Options, ROOT_GROUP_ID, Rate, engine};
        let mut b = DspBuilder::new(1);
        let gate = gate(&mut b);
        let out = node.ugens(&[0], &[gate], &mut b)[0].channel(0).unwrap();
        let out_ins = vec![InputRef::Constant(0.0), out];
        b.push_unit(UnitSpec::new("Out", Rate::Audio, out_ins, 0));
        let (mut controller, _nrt, mut world) = engine(Options {
            sample_rate: SR as f64,
            output_channels: 1,
            ..Options::default()
        });
        controller.add_synthdef(b.finish("env").def);
        controller
            .synth_new("env", ROOT_GROUP_ID, AddAction::Tail)
            .expect("synth_new");
        let mut got = Vec::with_capacity(frames + 64);
        let mut block = vec![0.0f32; 64];
        while got.len() < frames {
            world.fill(&mut block, 1);
            got.extend_from_slice(&block);
        }
        got.truncate(frames);
        got
    }

    /// A `kr` impulse into the gate restarts a percussive envelope on each
    /// impulse.
    #[test]
    fn a_kr_impulse_retriggers_the_envelope() {
        use plyphon::Rate;
        let node = Envgen::new(Envelope::perc(0.001, 0.02));
        let impulse = |b: &mut DspBuilder| {
            let freq = vec![InputRef::Constant(10.0), InputRef::Constant(0.0)];
            let unit = b.push_unit(UnitSpec::new("Impulse", Rate::Control, freq, 1));
            Some(Signal::mono(InputRef::Unit { unit, output: 0 }))
        };
        let got = render(&node, impulse, (0.35 * SR) as usize);
        // Each envelope rises above 0.9 once. Count the rises.
        let rises = got.windows(2).filter(|w| w[0] <= 0.9 && w[1] > 0.9).count();
        assert_eq!(rises, 4, "one peak per impulse at 0, 0.1, 0.2 and 0.3 s");
    }

    /// The engine plays the envelope as `Envelope::level_at` draws it, for
    /// every shape. So the editor's plot shows what the node sounds like.
    #[test]
    fn the_engine_plays_level_at() {
        let seg = |level, shape| Segment::new(level, 0.01, shape);
        let env = Envelope {
            init: 0.1,
            segments: vec![
                Segment::curved(1.0, 0.01, -4.0),
                seg(0.5, Shape::Exp),
                seg(0.2, Shape::Sin),
                seg(0.8, Shape::Welch),
                seg(0.3, Shape::Squared),
                seg(0.6, Shape::Hold),
                seg(0.0, Shape::Lin),
            ],
            release: None,
        };
        let frames = (env.total_time() * SR) as usize + 256;
        let got = render(&Envgen::new(env.clone()), |_| None, frames);
        for (i, &sample) in got.iter().enumerate() {
            let expected = env.level_at(i as f32 / SR);
            assert!(
                (sample - expected).abs() < 1e-3,
                "sample {i}: the engine gives {sample}, level_at gives {expected}",
            );
        }
    }
}
