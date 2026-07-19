//! The [`NodeDsp`] trait, the [`Signal`] channel group a dsp port carries, the
//! [`DspBuilder`] that accumulates a synthdef, and the [`ToNodeDsp`] downcast
//! hook used to discover DSP nodes in an erased graph.

use plyphon::Rate;
use plyphon::synthdef::{InputRef, Param, SynthDef, UnitSpec};
use serde::{Deserialize, Serialize};

/// A dsp node's ugen rate: audio (`ar`, one value per sample) or control (`kr`,
/// one value per block - cheaper, for modulators). Structural: it sets the
/// emitted [`UnitSpec`]'s rate, so a change respawns the synth.
///
/// A consumer reading a control-rate wire at audio rate holds the value for the
/// whole block. Audio *sinks* (whose units read inputs strictly as audio, like
/// `Out`) lift control wires explicitly via [`DspBuilder::ensure_audio`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NodeRate {
    /// Audio rate (`ar`): one value per sample.
    #[default]
    #[serde(rename = "ar")]
    Audio,
    /// Control rate (`kr`): one value per block.
    #[serde(rename = "kr")]
    Control,
}

impl NodeRate {
    /// The plyphon [`Rate`] this maps to.
    pub fn to_plyphon(self) -> Rate {
        match self {
            NodeRate::Audio => Rate::Audio,
            NodeRate::Control => Rate::Control,
        }
    }

    /// The display / sugar token: `"ar"` or `"kr"`.
    pub fn token(self) -> &'static str {
        match self {
            NodeRate::Audio => "ar",
            NodeRate::Control => "kr",
        }
    }
}

/// A channel group: the mono wires a single dsp port carries.
///
/// A gantz signal edge is a channel-*group* wire (like SC's array signals, Max's
/// MC cords or VCV's poly cables): one edge carries [`width`](Self::width)
/// channels, lowered by the synthdef compiler to plyphon's strictly mono-wire
/// unit inputs (one [`InputRef`] per channel). A `Signal` is never empty -
/// silence is one channel of constant `0.0`, not a zero-channel group (plyphon
/// units reject empty input lists at synth-build time).
#[derive(Clone, Debug)]
pub struct Signal(Vec<InputRef>);

impl Signal {
    /// A single-channel signal from one wire.
    pub fn mono(input: InputRef) -> Self {
        Signal(vec![input])
    }

    /// `n` channels of silence (constant `0.0`). `n` is clamped to at least 1.
    pub fn silent(n: usize) -> Self {
        Signal(vec![InputRef::Constant(0.0); n.max(1)])
    }

    /// The number of channels this signal carries (always at least 1).
    pub fn width(&self) -> usize {
        self.0.len()
    }

    /// Channel `i`'s wire, or `None` past [`width`](Self::width).
    pub fn channel(&self, i: usize) -> Option<InputRef> {
        self.0.get(i).copied()
    }

    /// Iterate over the per-channel wires.
    pub fn channels(&self) -> impl Iterator<Item = InputRef> + '_ {
        self.0.iter().copied()
    }

    /// Concatenate channel groups into one wide group (width = the sum of the
    /// input widths). An empty iterator concatenates to mono silence.
    pub fn concat(signals: impl IntoIterator<Item = Signal>) -> Self {
        signals.into_iter().flat_map(|s| s.0).collect()
    }
}

impl FromIterator<InputRef> for Signal {
    /// Collect per-channel wires into a group. An empty iterator collects to
    /// mono silence (a `Signal` is never empty).
    fn from_iter<I: IntoIterator<Item = InputRef>>(iter: I) -> Self {
        let channels: Vec<InputRef> = iter.into_iter().collect();
        match channels.is_empty() {
            true => Signal::silent(1),
            false => Signal(channels),
        }
    }
}

/// The channel width and rate a dsp output port's [`Signal`] carried at derive
/// time (see [`PortShapes`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortShape {
    /// The number of channels the port carries.
    pub width: usize,
    /// The port's rate (see [`signal_rate`]).
    pub rate: Rate,
}

/// Per-port shapes recorded during derivation, keyed by
/// `(node path, dsp output port)`.
///
/// Covers exactly the ports derivation materialized a [`Signal`] for
/// (dsp-reachable nodes) - a port with no entry contributed nothing to the
/// derived program. A `BTreeMap` keeps any rendering of it deterministic.
pub type PortShapes = std::collections::BTreeMap<(Vec<usize>, usize), PortShape>;

/// A gantz node that contributes one or more plyphon UGens to a synthdef.
///
/// This is the audio/DSP analogue of [`gantz_core::Node`]: where `Node::expr`
/// emits control-rate Steel, [`NodeDsp::ugens`] emits plyphon [`UnitSpec`]s into
/// the synthdef under construction. A node is "DSP" simply by implementing this
/// trait (and being discoverable via [`ToNodeDsp`]). The same gantz graph is
/// compiled by both backends independently.
///
/// **Steel placeholder contract:** a dsp node's `Node::expr` output for a dsp
/// output port must not evaluate to a number (use `'()` or similar). Hybrid
/// control inputs ([`control_input_expr`](crate::param::control_input_expr))
/// distinguish a control value from an inert dsp edge with a `number?` guard, so
/// a numeric placeholder would be mistaken for a control value and stomp the
/// downstream node's param state. Nodes with no dsp outputs (e.g. `~scopeout`)
/// are exempt - their Steel outputs never feed a dsp edge.
pub trait NodeDsp {
    /// The number of DSP (signal) input *ports* - the leading inputs that carry
    /// signals, wired into the synthdef. A node's [`gantz_core::Node::n_inputs`]
    /// may exceed this: any inputs at indices `>= n_dsp_inputs` are *control*
    /// inputs, a purely Steel/state concern (a connected control value is written
    /// into the node's param state by its `expr`), and are ignored by the
    /// synthdef compiler.
    ///
    /// A dsp input may also be *hybrid*: backed by a control param it falls
    /// back to when no dsp source is connected (e.g. `~sinosc`'s freq). The two
    /// sides compose without coordination: the synthdef compiler only wires dsp
    /// sources (a connected number materializes no signal, so
    /// [`ugens`](Self::ugens) sees `None` and bakes the param), while the
    /// node's Steel `expr` ([`control_input_expr`](crate::param::control_input_expr))
    /// writes connected numbers into the param state and ignores dsp
    /// placeholders via its `number?` guard.
    fn n_dsp_inputs(&self) -> usize {
        0
    }

    /// The number of DSP (signal) output *ports*. Each port carries a whole
    /// channel group ([`Signal`]) - this counts ports, not channels. May differ
    /// from [`gantz_core::Node::n_outputs`] (e.g. `~scopeout` has two Steel
    /// outputs but no dsp outputs).
    fn n_dsp_outputs(&self) -> usize {
        1
    }

    /// Whether this node is a synthdef *sink* (e.g. `~out`) that the compiler
    /// uses as a root when deriving a synthdef.
    fn is_output(&self) -> bool {
        false
    }

    /// Whether this node is a synthdef *monitor* (e.g. `~scopeout`) - a sink that
    /// reads its dsp input back to the control world rather than to the speakers.
    /// Like [`is_output`](Self::is_output) it roots a synthdef pull, but instead
    /// of an `Out` it emits a `ScopeOut` (via [`DspBuilder::push_monitor`]) whose
    /// samples the audio driver streams into the node's VM state.
    fn is_monitor(&self) -> bool {
        false
    }

    /// Whether this node is a synthdef *boundary* (e.g. `~bus`): the multi-def
    /// compiler ([`derive_synthdefs`](crate::derive_synthdefs)) cuts the graph
    /// into per-region synthdefs here, lowering the boundary to a private-bus
    /// `Out`/`In` pair. Boundary nodes must have exactly one dsp input and one
    /// dsp output. Their [`ugens`](Self::ugens) is only invoked when both sides
    /// land in the same region (no cut) and should pass the signal through.
    fn is_boundary(&self) -> bool {
        false
    }

    /// Emit this node's UGens into `b`, given the resolved [`Signal`] for each
    /// DSP input port, returning one [`Signal`] per DSP output port (so
    /// downstream nodes can reference them).
    ///
    /// `path` is the node's path within the graph (e.g. `[2]` for the node at
    /// index 2 of a flat graph). Use it to name any control [`Param`]s
    /// uniquely within the synthdef (see [`param_name`](crate::param::param_name)).
    /// `inputs` has length [`n_dsp_inputs`](Self::n_dsp_inputs). A connected
    /// input arrives pre-summed as `Some` (a multi-edge input is the unity-gain
    /// mix of its summands, [`sum_signals`]). `None` means no dsp summand
    /// materialized a signal: the input is unconnected, or fed only by
    /// signal-less sources (e.g. a dangling `~unpack` port). A node may treat
    /// `None` as mono silence ([`input_or_silent`]) or fall back to a control
    /// param (a *hybrid* input). Params should broadcast across an input's
    /// channels (e.g. `~lag` emits one `Lag` unit per channel, all sharing the
    /// one `dur` param).
    fn ugens(&self, path: &[usize], inputs: &[Option<Signal>], b: &mut DspBuilder) -> Vec<Signal>;
}

/// A downcast hook so the synthdef compiler and the audio driver can find
/// [`NodeDsp`] nodes inside an erased node type (e.g. the erased UI node,
/// `gantz_egui::node::DynNode`).
///
/// Implemented per concrete DSP node type (returning `Some(self)`), and for
/// the erased UI node by trying each known DSP node type via [`node_dsp_of`]
/// (see the `egui` module). (A blanket `impl<T: NodeDsp>` is deliberately
/// avoided so the erased-node impl does not collide with it.)
pub trait ToNodeDsp {
    /// This value as a [`NodeDsp`], if it is one.
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp>;

    /// The node's path, used to name control [`Param`]s, key driver bindings
    /// (see [`ParamBinding::node_path`]) and hash region keys. `ix` is the
    /// node's index within the graph being derived.
    ///
    /// Defaults to `[ix]`, correct for a flat graph. The flattening pass
    /// (see [`flatten`](crate::flatten())) overrides this on its
    /// [`Flat`](crate::flatten::Flat) wrapper to return the node's original
    /// path within the nested structure, so params keep bridging to the
    /// node's VM state and identities stay stable across re-derives.
    fn node_path(&self, ix: usize) -> Vec<usize> {
        vec![ix]
    }
}

// References probe through to the referent, letting borrowed graphs (e.g. the
// flattening pass's `Flat<&N>` weights) derive without cloning nodes.
impl<T: ToNodeDsp + ?Sized> ToNodeDsp for &T {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        (**self).to_node_dsp()
    }

    fn node_path(&self, ix: usize) -> Vec<usize> {
        (**self).node_path(ix)
    }
}

/// Records which dsp node a synthdef [`Param`] came from, so the audio driver can
/// map a node's live state value to the right synth param index.
#[derive(Clone, Debug)]
pub struct ParamBinding {
    /// The dsp node's path within the graph (e.g. `[2]` for a flat graph).
    pub node_path: Vec<usize>,
    /// The param's index within the synthdef's `params`.
    pub index: usize,
}

/// The smoothing lag (seconds) of a driver-controlled fade gain - the ramp time
/// of each half of a crossfaded synth replacement. Long enough that the
/// `LagControl`'s per-control-tick steps stay small (no zipper), short enough
/// that edits feel immediate.
pub const FADE_LAG: f32 = 0.05;

/// Records a synthdef *fade gain* - a driver-owned param scaling a sink's whole
/// output - so the audio driver can fade the synth in and out across a
/// crossfaded replacement (the respawn de-click). The default is baked at
/// `0.0` so the synth spawns silent without any def mutation, and the driver
/// ramps it via the param's own `LagControl` - to `1.0` once the synth is up,
/// to `0.0` ahead of a deferred free. [`structural_sig`](crate::structural_sig)
/// excludes defaults, so the baked `0.0` does not churn the sig. Fade gains
/// have NO [`ParamBinding`]: no node state feeds them, the driver alone drives
/// them.
#[derive(Clone, Copy, Debug)]
pub struct GainRef {
    /// The param's index within the synthdef's `params`.
    pub index: usize,
    /// The param's smoothing lag in seconds - the fade's ramp time.
    pub lag: f32,
}

/// Records a monitor (`~scopeout`) node's `ScopeOut`, so the audio driver can cue a live
/// scope stream and route its samples into the right node's ring-buffer state,
/// capped at `size`. The `ScopeOut`'s `bufnum` is a no-lag control param in the
/// derived def; the driver allocates a globally-unique cued index and sets it
/// via `set_control` after spawning (no def mutation).
#[derive(Clone, Debug)]
pub struct ScopeOutBinding {
    /// The monitor node's path within the graph (where its ring state lives).
    pub node_path: Vec<usize>,
    /// The ring buffer length (frames) the driver caps each per-channel ring at.
    pub size: usize,
    /// The number of channels the `ScopeOut` streams (`cue_scope`'s width) -
    /// the width of the monitored input [`Signal`], inferred at derive time.
    pub channels: usize,
    /// The index within the def's `units` of this monitor's `ScopeOut`.
    pub scope_unit: usize,
    /// The no-lag control param the driver sets to the cued scope-stream index
    /// via `set_control` after spawning.
    pub bufnum_param: usize,
}

/// Records a buffer-playing node's (`~playbuf`) asset reference, so the audio
/// driver can make the referenced [`AssetAddr`](gantz_ca::ContentAddr) resident,
/// allocate a bufnum, install the buffer, and set the node's `bufnum`/`rate`
/// params after spawning.
///
/// The buffer analogue of [`ScopeOutBinding`], but where a scope stream is
/// per-synth read-back, a resident buffer is shared read-only across every
/// synth referencing the same asset (loaded once, refcounted). `bufnum_param`
/// and `rate_param` are no-lag control params (see
/// [`push_control_param`](DspBuilder::push_control_param)) the driver sets via
/// `set_control` after spawning (no def mutation) - the rate to
/// `sample_rate / engine_sample_rate` so `PlayBuf` advances at the right pitch.
#[derive(Clone, Debug)]
pub struct BufferBinding {
    /// The buffer node's path within the graph (where its state lives).
    pub node_path: Vec<usize>,
    /// The content-addressed audio asset the node plays.
    pub asset: gantz_ca::ContentAddr,
    /// The no-lag control param the driver sets to the resident bufnum.
    pub bufnum_param: usize,
    /// The no-lag control param the driver sets to the playback rate.
    pub rate_param: usize,
    /// The asset's own sample rate, for the driver's rate correction.
    pub sample_rate: f64,
}

/// The finished output of a [`DspBuilder`]: the compiled synthdef plus the
/// bindings the audio driver uses to bridge node state and the running synth.
pub struct Finished {
    /// The compiled synth definition.
    pub def: SynthDef,
    /// One binding per control param, in param-index order.
    pub params: Vec<ParamBinding>,
    /// One binding per monitor (`~scopeout`).
    pub monitors: Vec<ScopeOutBinding>,
    /// The params gating the def's whole output (fade gains).
    pub gains: Vec<GainRef>,
    /// One binding per buffer reference (`~playbuf`).
    pub buffers: Vec<BufferBinding>,
}

/// Accumulates the [`UnitSpec`]s and [`Param`]s of a synthdef as nodes emit them.
///
/// Also carries the engine's output-channel count so a sink node (`~out`) can
/// fan a mono signal across every output channel, and records a [`ParamBinding`]
/// per pushed param.
pub struct DspBuilder {
    units: Vec<UnitSpec>,
    params: Vec<Param>,
    bindings: Vec<ParamBinding>,
    monitors: Vec<ScopeOutBinding>,
    gains: Vec<GainRef>,
    buffers: Vec<BufferBinding>,
    out_channels: usize,
}

impl DspBuilder {
    /// A new, empty builder targeting `out_channels` output-bus channels.
    pub fn new(out_channels: usize) -> Self {
        DspBuilder {
            units: Vec::new(),
            params: Vec::new(),
            bindings: Vec::new(),
            monitors: Vec::new(),
            gains: Vec::new(),
            buffers: Vec::new(),
            out_channels: out_channels.max(1),
        }
    }

    /// Push a unit, returning its index for use in [`InputRef::Unit`].
    pub fn push_unit(&mut self, spec: UnitSpec) -> u32 {
        let ix = self.units.len() as u32;
        self.units.push(spec);
        ix
    }

    /// Declare a control parameter belonging to the dsp node at `path`, returning
    /// its index for [`InputRef::Param`] and recording its [`ParamBinding`].
    pub fn push_param(&mut self, path: &[usize], param: Param) -> u32 {
        let index = self.params.len();
        self.params.push(param);
        self.bindings.push(ParamBinding {
            node_path: path.to_vec(),
            index,
        });
        index as u32
    }

    /// Declare a driver-owned, no-lag control param (no [`ParamBinding`]),
    /// returning its index for [`InputRef::Param`]. Used for per-instance
    /// wiring - bus indices, scope bufnums - that the driver sets via
    /// `set_control` after spawning. No lag, since a lagged bus index would
    /// glide through wrong buses; the default is `0.0`.
    pub fn push_control_param(&mut self, path: &[usize], label: &str) -> u32 {
        let index = self.params.len();
        self.params
            .push(Param::control(crate::param::param_name(path, label), 0.0));
        index as u32
    }

    /// Declare a driver-controlled *fade gain* for the sink at `path`: a lagged
    /// param (default `0.0`, [`FADE_LAG`] ramp) that must scale the sink's whole
    /// output, recorded as a [`GainRef`] but with NO [`ParamBinding`] - node
    /// state never feeds it. The default is baked at `0.0` so the synth spawns
    /// silent; the driver ramps it to `1.0` via `set_control` once the synth is
    /// up (and to `0.0` ahead of a deferred free). Returns the param's index
    /// for [`InputRef::Param`].
    pub fn push_fade_gain(&mut self, path: &[usize]) -> u32 {
        let index = self.params.len();
        self.params.push(Param::lag(
            crate::param::param_name(path, "fade"),
            0.0,
            FADE_LAG,
        ));
        self.gains.push(GainRef {
            index,
            lag: FADE_LAG,
        });
        index as u32
    }

    /// Declare a monitor for the dsp node at `path`, recording its [`ScopeOutBinding`]
    /// so the driver can cue a `channels`-wide scope stream and route its samples into
    /// the node's ring state (capped at `size` frames). `scope_unit` is the index of the
    /// node's `ScopeOut` unit (from [`push_unit`](Self::push_unit)); `bufnum_param` is
    /// the no-lag control param the driver sets to the cued scope-stream index via
    /// `set_control` after spawning.
    pub fn push_monitor(
        &mut self,
        path: &[usize],
        size: usize,
        channels: usize,
        scope_unit: usize,
        bufnum_param: usize,
    ) {
        self.monitors.push(ScopeOutBinding {
            node_path: path.to_vec(),
            size,
            channels,
            scope_unit,
            bufnum_param,
        });
    }

    /// Declare a buffer reference for the dsp node at `path`, recording a
    /// [`BufferBinding`] so the driver can make `asset` resident and set the
    /// node's `bufnum`/`rate` params after spawning. `bufnum_param` and
    /// `rate_param` are no-lag control params (from
    /// [`push_control_param`](Self::push_control_param)); `sample_rate` is the
    /// asset's own rate, which the driver divides by the engine rate to set
    /// `rate`.
    pub fn push_buffer(
        &mut self,
        path: &[usize],
        asset: gantz_ca::ContentAddr,
        bufnum_param: u32,
        rate_param: u32,
        sample_rate: f64,
    ) {
        self.buffers.push(BufferBinding {
            node_path: path.to_vec(),
            asset,
            bufnum_param: bufnum_param as usize,
            rate_param: rate_param as usize,
            sample_rate,
        });
    }

    /// The number of output-bus channels a sink should fan its signal across.
    pub fn out_channels(&self) -> usize {
        self.out_channels
    }

    /// The rate of the wire behind `input`: a unit output takes its unit's rate,
    /// a param its param rate, and a constant literal is scalar.
    pub fn input_rate(&self, input: &InputRef) -> Rate {
        match input {
            InputRef::Constant(_) => Rate::Scalar,
            InputRef::Param(i) => self.params[*i as usize].rate,
            InputRef::Unit { unit, .. } => self.units[*unit as usize].rate,
        }
    }

    /// Lift `ch` to an audio-rate wire: the identity for an audio wire or a
    /// constant literal (consumers fold constants natively), else a `K2A`
    /// (control-to-audio conversion, ramping from the previous block's value).
    ///
    /// Audio *sinks* need it: a unit that reads its inputs strictly as audio
    /// (`Out`'s channels, for example) sees a control- or scalar-rate wire as
    /// SILENCE, not a held value.
    pub fn ensure_audio(&mut self, ch: InputRef) -> InputRef {
        match ch {
            InputRef::Constant(_) => ch,
            _ if matches!(self.input_rate(&ch), Rate::Audio) => ch,
            _ => {
                let unit = self.push_unit(UnitSpec::new("K2A", Rate::Audio, vec![ch], 1));
                InputRef::Unit { unit, output: 0 }
            }
        }
    }

    /// Consume the builder into a [`Finished`] synthdef and its param, monitor,
    /// gain and buffer bindings.
    pub fn finish(self, name: impl Into<String>) -> Finished {
        let def = SynthDef {
            name: name.into(),
            params: self.params,
            units: self.units,
        };
        Finished {
            def,
            params: self.bindings,
            monitors: self.monitors,
            gains: self.gains,
            buffers: self.buffers,
        }
    }
}

/// Find the [`NodeDsp`] within a type-erased node, trying each of this crate's
/// DSP node types.
///
/// Node-set types (e.g. an app's `Box<dyn Node>`) can implement [`ToNodeDsp`]
/// by delegating to this fn. Sets composing additional DSP node types chain
/// their own downcasts via `.or_else(..)`.
///
/// Keep the probe list in step with `crate::ref_ext`'s data-level DSP tag
/// set, which classifies the same types by wire tag.
pub fn node_dsp_of(any: &dyn std::any::Any) -> Option<&dyn NodeDsp> {
    fn probe<T: NodeDsp + 'static>(any: &dyn std::any::Any) -> Option<&dyn NodeDsp> {
        any.downcast_ref::<T>().map(|n| n as &dyn NodeDsp)
    }
    probe::<crate::SinOsc>(any)
        .or_else(|| probe::<crate::Out>(any))
        .or_else(|| probe::<crate::Lag>(any))
        .or_else(|| probe::<crate::ScopeOut>(any))
        .or_else(|| probe::<crate::Pack>(any))
        .or_else(|| probe::<crate::Sum>(any))
        .or_else(|| probe::<crate::Unpack>(any))
        .or_else(|| probe::<crate::Bus>(any))
        .or_else(|| probe::<crate::PlayBuf>(any))
}

/// The signal at dsp input `i` of a [`NodeDsp::ugens`] `inputs` slice, or mono
/// silence when no signal materialized there - the common fallback for a
/// non-hybrid input.
pub fn input_or_silent(inputs: &[Option<Signal>], i: usize) -> Signal {
    inputs
        .get(i)
        .cloned()
        .flatten()
        .unwrap_or_else(|| Signal::silent(1))
}

/// The rate of a channel group: audio if any channel is audio, else control
/// if any is control, else scalar (a constant-only signal, e.g. baked
/// silence).
pub fn signal_rate(b: &DspBuilder, sig: &Signal) -> Rate {
    let any = |rate: Rate| sig.channels().any(|ch| b.input_rate(&ch) == rate);
    if any(Rate::Audio) {
        Rate::Audio
    } else if any(Rate::Control) {
        Rate::Control
    } else {
        Rate::Scalar
    }
}

/// Record one [`PortShape`] per output port of the node at `path` (with output
/// [`Signal`]s `outs`) into `shapes`.
pub(crate) fn record_port_shapes(
    shapes: &mut PortShapes,
    b: &DspBuilder,
    path: &[usize],
    outs: &[Signal],
) {
    let shape = |sig| PortShape {
        width: Signal::width(sig),
        rate: signal_rate(b, sig),
    };
    let entries = outs
        .iter()
        .enumerate()
        .map(|(port, sig)| ((path.to_vec(), port), shape(sig)));
    shapes.extend(entries);
}

/// Sum channel groups into one group: the unity-gain mix of every summand.
///
/// The result's width is the widest summand's. A mono summand broadcasts its
/// single channel into every result channel. A wider-but-narrower summand
/// contributes silence past its own width. Constant channels fold at derive
/// time (so silent placeholders vanish). No summands sum to mono silence, and
/// a lone summand passes through untouched (zero units), keeping a
/// single-edge input's derive byte-identical to a direct wire.
pub fn sum_signals(b: &mut DspBuilder, signals: &[Signal]) -> Signal {
    match signals {
        [] => Signal::silent(1),
        [s] => s.clone(),
        _ => {
            let width = signals.iter().map(Signal::width).max().unwrap_or(1);
            (0..width).map(|ch| sum_channel(b, signals, ch)).collect()
        }
    }
}

/// The wire carrying channel `ch` of the sum of `signals`: each summand
/// contributes its channel `ch`, a mono summand its broadcast channel `0`, a
/// wider-but-narrower summand nothing. Constant contributions fold into one
/// trailing term, dropped when zero and other wires remain.
fn sum_channel(b: &mut DspBuilder, signals: &[Signal], ch: usize) -> InputRef {
    let mut constant = 0.0;
    let mut wires = Vec::new();
    let contributions = signals.iter().filter_map(|s| match s.width() {
        1 => s.channel(0),
        _ => s.channel(ch),
    });
    for c in contributions {
        match c {
            InputRef::Constant(v) => constant += v,
            wire => wires.push(wire),
        }
    }
    if constant != 0.0 || wires.is_empty() {
        wires.push(InputRef::Constant(constant));
    }
    sum_wires(b, wires)
}

/// Sum a non-empty list of mono wires, tiling plyphon's summing units: one
/// wire passes through, two add via a `BinaryOpUGen`, three or four via
/// `Sum3`/`Sum4` (strict arity - `SumCtor` rejects a padded input list), and
/// more tile as a `Sum4` over the first four fed back as the leading summand
/// of the rest.
fn sum_wires(b: &mut DspBuilder, mut wires: Vec<InputRef>) -> InputRef {
    while wires.len() > 4 {
        let head: Vec<InputRef> = wires.drain(..4).collect();
        let sum = push_sum_unit(b, head);
        wires.insert(0, sum);
    }
    match wires.len() {
        1 => wires[0],
        _ => push_sum_unit(b, wires),
    }
}

/// Emit one summing unit over `inputs` (2 -> `BinaryOpUGen` add, 3 -> `Sum3`,
/// 4 -> `Sum4`): audio rate if any input is audio, else control rate. Each
/// input is still read at its own rate.
fn push_sum_unit(b: &mut DspBuilder, inputs: Vec<InputRef>) -> InputRef {
    let audio = inputs
        .iter()
        .any(|i| matches!(b.input_rate(i), Rate::Audio));
    let rate = match audio {
        true => Rate::Audio,
        false => Rate::Control,
    };
    let name = match inputs.len() {
        2 => "BinaryOpUGen",
        3 => "Sum3",
        _ => "Sum4",
    };
    let unit = b.push_unit(UnitSpec::new(name, rate, inputs, 1));
    InputRef::Unit { unit, output: 0 }
}

#[cfg(test)]
mod tests {
    use plyphon::Rate;
    use plyphon::synthdef::{InputRef, UnitSpec};

    use super::{DspBuilder, Signal, node_dsp_of, sum_signals};

    /// Every DSP node type in this crate must be found by [`node_dsp_of`], so
    /// a probe arm forgotten when adding a node fails here rather than in
    /// downstream node sets.
    #[test]
    fn node_dsp_of_covers_all_dsp_nodes() {
        fn check<T: super::NodeDsp + Default + 'static>() {
            let node = T::default();
            assert!(node_dsp_of(&node).is_some());
        }
        check::<crate::SinOsc>();
        check::<crate::Out>();
        check::<crate::Lag>();
        check::<crate::ScopeOut>();
        check::<crate::Pack>();
        check::<crate::Sum>();
        check::<crate::Unpack>();
        check::<crate::Bus>();
        check::<crate::PlayBuf>();
    }

    /// A unit-backed mono wire at `rate` to feed the summing helpers.
    fn wire(b: &mut DspBuilder, rate: Rate) -> InputRef {
        let unit = b.push_unit(UnitSpec::new("SinOsc", rate, vec![], 1));
        InputRef::Unit { unit, output: 0 }
    }

    /// The names of the units pushed at or after index `from`.
    fn unit_names(b: &DspBuilder, from: usize) -> Vec<String> {
        b.units[from..].iter().map(|u| u.name.clone()).collect()
    }

    /// `InputRef` derives no `PartialEq`; compare wires via `Debug`.
    fn wire_eq(a: &InputRef, b: &InputRef) -> bool {
        format!("{a:?}") == format!("{b:?}")
    }

    #[test]
    fn sum_of_none_is_mono_silence() {
        let mut b = DspBuilder::new(2);
        let sum = sum_signals(&mut b, &[]);
        assert_eq!(sum.width(), 1);
        assert!(wire_eq(&sum.channel(0).unwrap(), &InputRef::Constant(0.0)));
        assert!(b.units.is_empty());
    }

    #[test]
    fn sum_of_one_passes_through_unit_free() {
        let mut b = DspBuilder::new(2);
        let w0 = wire(&mut b, Rate::Audio);
        let w1 = wire(&mut b, Rate::Audio);
        let stereo: Signal = [w0, w1].into_iter().collect();
        let before = b.units.len();
        let sum = sum_signals(&mut b, &[stereo.clone()]);
        assert_eq!(b.units.len(), before);
        assert_eq!(sum.width(), 2);
        assert!(wire_eq(&sum.channel(0).unwrap(), &w0));
        assert!(wire_eq(&sum.channel(1).unwrap(), &w1));
    }

    #[test]
    fn sum_tiles_binary_sum3_sum4_and_chains() {
        for (n, expected) in [
            (2, vec!["BinaryOpUGen"]),
            (3, vec!["Sum3"]),
            (4, vec!["Sum4"]),
            (5, vec!["Sum4", "BinaryOpUGen"]),
            (9, vec!["Sum4", "Sum4", "Sum3"]),
        ] {
            let mut b = DspBuilder::new(2);
            let signals: Vec<Signal> = (0..n)
                .map(|_| Signal::mono(wire(&mut b, Rate::Audio)))
                .collect();
            let before = b.units.len();
            let sum = sum_signals(&mut b, &signals);
            assert_eq!(sum.width(), 1);
            assert_eq!(unit_names(&b, before), expected, "n = {n}");
            // An add is special_index 0 (a `BinaryOpUGen` selector, unset on
            // `Sum3`/`Sum4`).
            assert!(b.units[before..].iter().all(|u| u.special_index == 0));
        }
    }

    #[test]
    fn mono_broadcasts_into_every_channel() {
        let mut b = DspBuilder::new(2);
        let m = wire(&mut b, Rate::Audio);
        let s0 = wire(&mut b, Rate::Audio);
        let s1 = wire(&mut b, Rate::Audio);
        let stereo: Signal = [s0, s1].into_iter().collect();
        let sum = sum_signals(&mut b, &[Signal::mono(m), stereo]);
        assert_eq!(sum.width(), 2);
        for (ch, s) in [(0, s0), (1, s1)] {
            let InputRef::Unit { unit, .. } = sum.channel(ch).unwrap() else {
                panic!("channel {ch} is not a summing unit");
            };
            let inputs = &b.units[unit as usize].inputs;
            assert!(inputs.iter().any(|i| wire_eq(i, &m)));
            assert!(inputs.iter().any(|i| wire_eq(i, &s)));
        }
    }

    #[test]
    fn narrower_summand_contributes_silence_past_its_width() {
        let mut b = DspBuilder::new(2);
        let s0 = wire(&mut b, Rate::Audio);
        let s1 = wire(&mut b, Rate::Audio);
        let w0 = wire(&mut b, Rate::Audio);
        let w1 = wire(&mut b, Rate::Audio);
        let w2 = wire(&mut b, Rate::Audio);
        let stereo: Signal = [s0, s1].into_iter().collect();
        let wide: Signal = [w0, w1, w2].into_iter().collect();
        let before = b.units.len();
        let sum = sum_signals(&mut b, &[stereo, wide]);
        assert_eq!(sum.width(), 3);
        // Channels 0 and 1 sum a pair; channel 2 is the wide summand's own
        // wire passed through (the stereo summand contributes nothing there).
        assert_eq!(unit_names(&b, before), vec!["BinaryOpUGen", "BinaryOpUGen"]);
        assert!(wire_eq(&sum.channel(2).unwrap(), &w2));
    }

    #[test]
    fn constants_fold_at_derive_time() {
        // Silence + a wire: the zero constant vanishes, the wire passes
        // through, no units.
        let mut b = DspBuilder::new(2);
        let w = wire(&mut b, Rate::Audio);
        let before = b.units.len();
        let sum = sum_signals(&mut b, &[Signal::silent(1), Signal::mono(w)]);
        assert_eq!(b.units.len(), before);
        assert!(wire_eq(&sum.channel(0).unwrap(), &w));

        // Pure constants fold to one constant, no units.
        let mut b = DspBuilder::new(2);
        let sum = sum_signals(
            &mut b,
            &[
                Signal::mono(InputRef::Constant(1.5)),
                Signal::mono(InputRef::Constant(2.0)),
            ],
        );
        assert!(b.units.is_empty());
        assert!(wire_eq(&sum.channel(0).unwrap(), &InputRef::Constant(3.5)));

        // A non-zero folded constant joins the wires as one trailing summand.
        let mut b = DspBuilder::new(2);
        let w0 = wire(&mut b, Rate::Audio);
        let w1 = wire(&mut b, Rate::Audio);
        let before = b.units.len();
        let sum = sum_signals(
            &mut b,
            &[
                Signal::mono(w0),
                Signal::mono(InputRef::Constant(1.5)),
                Signal::mono(w1),
            ],
        );
        assert_eq!(unit_names(&b, before), vec!["Sum3"]);
        let InputRef::Unit { unit, .. } = sum.channel(0).unwrap() else {
            panic!("expected a summing unit");
        };
        let inputs = &b.units[unit as usize].inputs;
        assert!(inputs.iter().any(|i| wire_eq(i, &InputRef::Constant(1.5))));
    }

    #[test]
    fn sum_unit_rate_is_audio_iff_any_summand_is() {
        let mut b = DspBuilder::new(2);
        let k0 = wire(&mut b, Rate::Control);
        let k1 = wire(&mut b, Rate::Control);
        let before = b.units.len();
        sum_signals(&mut b, &[Signal::mono(k0), Signal::mono(k1)]);
        assert_eq!(b.units[before].rate, Rate::Control);

        let mut b = DspBuilder::new(2);
        let k = wire(&mut b, Rate::Control);
        let a = wire(&mut b, Rate::Audio);
        let before = b.units.len();
        sum_signals(&mut b, &[Signal::mono(k), Signal::mono(a)]);
        assert_eq!(b.units[before].rate, Rate::Audio);
    }
}
