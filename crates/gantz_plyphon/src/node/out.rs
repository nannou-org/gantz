//! The `~out` audio-output sink node.

use std::hash::{Hash, Hasher};

use gantz_ca::Datum;
use gantz_nodedef::{ExprArgs, Meta, View};
use gantz_nodetag::NodeTag;
use plyphon::Rate;
use plyphon::synthdef::{InputRef, UnitSpec};
use serde::{Deserialize, Serialize};

use crate::dsp::{DspBuilder, FadeSink, NodeDsp, Signal, ToNodeDsp, input_or_silent};
use crate::param::{control_input_expr, param_name, param_state, plyphon_param};
use crate::view::{LAG, MAX_LAG, doc, edit_secs, param_row};

/// The audio output sink. Applies a master `gain` to its input signal and
/// writes it to the output buses. A mono input is fanned across every device
/// channel. A wider input writes channel `i` to bus `i`. Excess channels are
/// dropped and a deficit leaves the upper device channels silent. The
/// compiler roots a synthdef at this node.
///
/// The `gain` value lives in the node's VM state, like `number`. Only the
/// smoothing `gain_lag` is in the weight. It is structural and a small
/// de-click by default.
#[derive(Clone, Debug, Serialize, Deserialize, NodeTag)]
pub struct Out {
    #[serde(
        default = "default_gain_lag",
        skip_serializing_if = "is_default_gain_lag"
    )]
    gain_lag: f32,
}

impl Out {
    /// The default master gain in linear amplitude a fresh `~out` starts at.
    pub const DEFAULT_GAIN: f32 = 0.2;

    /// The default gain smoothing lag in seconds, a short de-click.
    pub const DEFAULT_GAIN_LAG: f32 = 0.01;

    /// The gain smoothing lag in seconds. `0.0` is instant.
    pub fn gain_lag(&self) -> f32 {
        self.gain_lag
    }

    /// Set the gain smoothing lag in seconds. It affects the content address.
    pub fn set_gain_lag(&mut self, lag: f32) {
        self.gain_lag = lag;
    }
}

impl Default for Out {
    fn default() -> Self {
        Out {
            gain_lag: default_gain_lag(),
        }
    }
}

impl PartialEq for Out {
    fn eq(&self, other: &Self) -> bool {
        self.gain_lag.to_bits() == other.gain_lag.to_bits()
    }
}

impl Eq for Out {}

impl Hash for Out {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Hash::hash(&self.gain_lag.to_bits(), state);
    }
}

impl gantz_nodedef::NodeDef for Out {
    const TAG: &'static str = <Self as NodeTag>::TAG;

    fn meta(&self) -> Meta {
        Meta {
            name: "~out".into(),
            description: "Audio output: master gain to the speakers".into(),
            inputs: 2,
            outputs: 0,
            stateful: true,
            ..Meta::default()
        }
    }

    fn expr(&self, ctx: ExprArgs<'_>) -> Result<String, String> {
        // A 0-output sink. The audio input at index 0 is a dsp edge handled by
        // the synthdef and ignored here. When the gain control at index 1 is
        // connected, write it into state. The DSP runtime applies it via
        // `set_control`.
        Ok(control_input_expr(&ctx, self.n_dsp_inputs(), "'()"))
    }

    fn init_state(&self) -> Option<Datum> {
        Some(param_state(Self::DEFAULT_GAIN as f64))
    }

    fn view(&self) -> View {
        let signal = "signal to send to the audio output. Mono fans across every device \
                      channel, wider signals write channel i to bus i";
        let gain = "master gain control. Overrides the inspector value while connected";
        let value = vec![crate::param::VALUE.to_string()];
        View {
            inputs: vec![doc("signal", signal), doc("number", gain)],
            rows: vec![param_row("gain", value, [0.0, 1.0], "", self.gain_lag)],
            hide_state: true,
            ..View::default()
        }
    }

    fn edit(&self, edit: &str, value: &Datum) -> Option<Self> {
        let gain_lag =
            edit_secs(value, MAX_LAG).filter(|_| edit.strip_prefix(LAG) == Some("gain"))?;
        Some(Out { gain_lag })
    }
}

impl NodeDsp for Out {
    fn n_dsp_inputs(&self) -> usize {
        1
    }

    fn n_dsp_outputs(&self) -> usize {
        0
    }

    fn is_output(&self) -> bool {
        true
    }

    fn ugens(&self, path: &[usize], inputs: &[Option<Signal>], b: &mut DspBuilder) -> Vec<Signal> {
        let sig = input_or_silent(inputs, 0);
        let out_channels = b.out_channels();
        // The output level is gain times fade, multiplied once at control
        // rate. `gain` is the settable, smoothed control param. The driver
        // applies its live state value via `set_control`. `fade` ramps the
        // whole synth in from spawn, and the driver ramps it out across a
        // replacement, see `DspBuilder::push_fade_gain`.
        let gain = b.push_param(
            path,
            plyphon_param(param_name(path, "gain"), Self::DEFAULT_GAIN, self.gain_lag),
        );
        let fade = b.push_fade_gain(path, FadeSink::Output);
        let level = b.push_unit(UnitSpec {
            name: "BinaryOpUGen".to_string(),
            rate: Rate::Control,
            inputs: vec![InputRef::Param(gain), fade].into(),
            num_outputs: 1,
            special_index: 2,
        });
        let level = InputRef::Unit {
            unit: level,
            output: 0,
        };
        // Each written channel is `ch * level`, a `BinaryOpUGen` multiply at
        // special_index 2. A control-rate channel is lifted to audio first.
        // `Out.ar` reads its inputs strictly as audio, so a kr wire would be
        // silence, and multiplying without the K2A ramp would zipper.
        let gained = |b: &mut DspBuilder, ch: InputRef| {
            let ch = b.ensure_audio(ch);
            let unit = b.push_unit(UnitSpec {
                name: "BinaryOpUGen".to_string(),
                rate: Rate::Audio,
                inputs: vec![ch, level].into(),
                num_outputs: 1,
                special_index: 2,
            });
            InputRef::Unit { unit, output: 0 }
        };
        // `Out.ar(0, sigs)` takes the bus index followed by one signal input
        // per written device channel. A mono input fans across every device
        // channel. A wider input writes channel `i` to bus `i` for the first
        // `min(width, out_channels)` channels. Excess input channels are
        // dropped, not summed or wrapped. A deficit leaves the upper device
        // channels silent. Only written channels get a gain multiply, since
        // dead units would pollute the structural sig.
        let mut out_inputs = vec![InputRef::Constant(0.0)];
        if sig.width() == 1 {
            let ch = gained(b, sig.channel(0).expect("a signal is never empty"));
            out_inputs.extend(std::iter::repeat_n(ch, out_channels));
        } else {
            let channels: Vec<InputRef> = sig.channels().take(out_channels).collect();
            out_inputs.extend(channels.into_iter().map(|ch| gained(b, ch)));
        }
        b.push_unit(UnitSpec::new("Out", Rate::Audio, out_inputs, 0));
        vec![]
    }
}

impl ToNodeDsp for Out {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        Some(self)
    }
}

fn default_gain_lag() -> f32 {
    Out::DEFAULT_GAIN_LAG
}

fn is_default_gain_lag(gain_lag: &f32) -> bool {
    *gain_lag == default_gain_lag()
}
