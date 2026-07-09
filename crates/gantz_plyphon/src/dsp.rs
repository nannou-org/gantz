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

/// Fold a node's ugen `rate` into a content-address hasher, but only when
/// non-default (audio) - so existing audio-rate nodes keep their addresses.
pub fn cahash_rate(hasher: &mut gantz_ca::Hasher, rate: NodeRate) {
    if rate != NodeRate::Audio {
        hasher.update(b"rate");
        hasher.update(rate.token().as_bytes());
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

/// A gantz node that contributes one or more plyphon UGens to a synthdef.
///
/// This is the audio/DSP analogue of [`gantz_core::Node`]: where `Node::expr`
/// emits control-rate Steel, [`NodeDsp::ugens`] emits plyphon [`UnitSpec`]s into
/// the synthdef under construction. A node is "DSP" simply by implementing this
/// trait (and being discoverable via [`ToNodeDsp`]). The same gantz graph is
/// compiled by both backends independently.
pub trait NodeDsp {
    /// The number of DSP (signal) input *ports* - the leading inputs that carry
    /// signals, wired into the synthdef. A node's [`gantz_core::Node::n_inputs`]
    /// may exceed this: any inputs at indices `>= n_dsp_inputs` are *control*
    /// inputs, a purely Steel/state concern (a connected control value is written
    /// into the node's param state by its `expr`), and are ignored by the
    /// synthdef compiler.
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
    /// `inputs` has length [`n_dsp_inputs`](Self::n_dsp_inputs). An unconnected
    /// input is [`Signal::silent`]`(1)` (mono silence). Params should broadcast
    /// across an input's channels (e.g. `~lag` emits one `Lag` unit per channel,
    /// all sharing the one `dur` param).
    fn ugens(&self, path: &[usize], inputs: &[Signal], b: &mut DspBuilder) -> Vec<Signal>;
}

/// A downcast hook so the synthdef compiler and the audio driver can find
/// [`NodeDsp`] nodes inside an erased node type (e.g. `Box<dyn Node>`).
///
/// Implemented per concrete DSP node type (returning `Some(self)`). The
/// application implements it for its boxed node enum by trying each known DSP
/// node type - mirroring `ToTickBang` in `bevy_gantz_egui`. (A blanket
/// `impl<T: NodeDsp>` is deliberately avoided so the application's
/// `impl ToNodeDsp for Box<dyn Node>` does not collide with it.)
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
/// crossfaded replacement (the respawn de-click). The driver patches the
/// param's `default` from `1.0` to `0.0` before install (the synth spawns
/// silent. [`structural_sig`](crate::structural_sig) excludes defaults) and
/// ramps it via the param's own `LagControl` - to `1.0` once the synth is up,
/// to `0.0` ahead of a deferred free. Fade gains have NO [`ParamBinding`]: no
/// node state feeds them, the driver alone drives them.
#[derive(Clone, Copy, Debug)]
pub struct GainRef {
    /// The param's index within the synthdef's `params`.
    pub index: usize,
    /// The param's smoothing lag in seconds - the fade's ramp time.
    pub lag: f32,
}

/// Records a monitor (`~scopeout`) node's `ScopeOut`, so the audio driver can cue a live
/// scope stream and route its samples into the right node's ring-buffer state,
/// capped at `size`. The `ScopeOut`'s `bufnum` input is a placeholder in the derived
/// def. The driver allocates a globally-unique cued index and patches the unit at
/// `scope_unit` before installing the def.
#[derive(Clone, Debug)]
pub struct ScopeOutBinding {
    /// The monitor node's path within the graph (where its ring state lives).
    pub node_path: Vec<usize>,
    /// The ring buffer length (frames) the driver caps each per-channel ring at.
    pub size: usize,
    /// The number of channels the `ScopeOut` streams (`cue_scope`'s width) -
    /// the width of the monitored input [`Signal`], inferred at derive time.
    pub channels: usize,
    /// The index within the def's `units` of this monitor's `ScopeOut`, so the driver
    /// can patch its `bufnum` (input 0) to the cued scope-stream index.
    pub scope_unit: usize,
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

    /// Declare a driver-controlled *fade gain* for the sink at `path`: a lagged
    /// param (default `1.0`, [`FADE_LAG`] ramp) that must scale the sink's whole
    /// output, recorded as a [`GainRef`] but with NO [`ParamBinding`] - node
    /// state never feeds it. the driver alone drives it across a crossfaded
    /// replacement. Returns the param's index for [`InputRef::Param`].
    pub fn push_fade_gain(&mut self, path: &[usize]) -> u32 {
        let index = self.params.len();
        self.params.push(Param::lag(
            crate::param::param_name(path, "fade"),
            1.0,
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
    /// node's `ScopeOut` unit (from [`push_unit`](Self::push_unit)), so the driver can
    /// patch its `bufnum`.
    pub fn push_monitor(
        &mut self,
        path: &[usize],
        size: usize,
        channels: usize,
        scope_unit: usize,
    ) {
        self.monitors.push(ScopeOutBinding {
            node_path: path.to_vec(),
            size,
            channels,
            scope_unit,
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

    /// Consume the builder into a finished [`SynthDef`], its param bindings, its
    /// monitor bindings, and its gain refs.
    pub fn finish(
        self,
        name: impl Into<String>,
    ) -> (
        SynthDef,
        Vec<ParamBinding>,
        Vec<ScopeOutBinding>,
        Vec<GainRef>,
    ) {
        let def = SynthDef {
            name: name.into(),
            params: self.params,
            units: self.units,
        };
        (def, self.bindings, self.monitors, self.gains)
    }
}

/// Find the [`NodeDsp`] within a type-erased node, trying each of this crate's
/// DSP node types.
///
/// Node-set types (e.g. an app's `Box<dyn Node>`) can implement [`ToNodeDsp`]
/// by delegating to this fn. Sets composing additional DSP node types chain
/// their own downcasts via `.or_else(..)`.
pub fn node_dsp_of(any: &dyn std::any::Any) -> Option<&dyn NodeDsp> {
    fn probe<T: NodeDsp + 'static>(any: &dyn std::any::Any) -> Option<&dyn NodeDsp> {
        any.downcast_ref::<T>().map(|n| n as &dyn NodeDsp)
    }
    probe::<crate::SinOsc>(any)
        .or_else(|| probe::<crate::Out>(any))
        .or_else(|| probe::<crate::Lag>(any))
        .or_else(|| probe::<crate::ScopeOut>(any))
        .or_else(|| probe::<crate::Pack>(any))
        .or_else(|| probe::<crate::Unpack>(any))
        .or_else(|| probe::<crate::Bus>(any))
}

#[cfg(test)]
mod tests {
    use super::node_dsp_of;

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
        check::<crate::Unpack>();
        check::<crate::Bus>();
    }
}
