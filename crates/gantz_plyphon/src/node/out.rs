//! The `~out` audio-output sink node.

use std::hash::{Hash, Hasher};

use gantz_core::node::{ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_nodetag::NodeTag;
use plyphon::Rate;
use plyphon::synthdef::{InputRef, UnitSpec};
use serde::{Deserialize, Serialize};

use crate::dsp::{DspBuilder, NodeDsp, Signal, ToNodeDsp, input_or_silent};
use crate::param::{control_input_expr, param_name, param_state, plyphon_param};

/// The audio output sink. Applies a master `gain` to its input signal and writes
/// it to the output buses. A mono input is fanned across every device channel; a
/// wider input writes channel `i` to bus `i` (excess channels are dropped, a
/// deficit leaves the upper device channels silent). The compiler roots a
/// synthdef at this node.
///
/// The `gain` *value* lives in the node's VM state (like `number`); only the
/// smoothing `gain_lag` (structural; a small de-click by default) is in the weight.
#[derive(Clone, Debug, Serialize, Deserialize, NodeTag)]
pub struct Out {
    #[serde(
        default = "default_gain_lag",
        skip_serializing_if = "is_default_gain_lag"
    )]
    gain_lag: f32,
}

impl Out {
    /// The default master gain (linear amplitude) a fresh `~out` starts at.
    pub const DEFAULT_GAIN: f32 = 0.2;

    /// The default gain smoothing lag in seconds (a short de-click).
    pub const DEFAULT_GAIN_LAG: f32 = 0.01;

    /// The gain smoothing lag in seconds (`0.0` = instant).
    pub fn gain_lag(&self) -> f32 {
        self.gain_lag
    }

    /// Set the gain smoothing lag in seconds (content-address affecting).
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

impl gantz_core::Node for Out {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        // Input 0 is the audio signal (a dsp edge); input 1 is the gain control.
        2
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        true
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        let path = ctx.path();
        gantz_core::node::state::init_value_if_absent(ctx.vm(), path, || {
            param_state(Self::DEFAULT_GAIN as f64)
        })
        .unwrap()
    }

    fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult {
        // A 0-output sink. The audio input (index 0) is a dsp edge handled by the
        // synthdef and ignored here; when the gain control (index 1) is connected,
        // write it into state (the audio driver applies it via `set_control`).
        control_input_expr(&ctx, self.n_dsp_inputs(), "'()")
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
        // The output level = gain x fade, multiplied once at control rate:
        // `gain` is the settable (smoothed) control param (the driver applies
        // its live state value via `set_control`); `fade` is the driver-owned
        // crossfade lever ramping the whole synth in and out across a
        // replacement (see `DspBuilder::push_fade_gain`).
        let gain = b.push_param(
            path,
            plyphon_param(param_name(path, "gain"), Self::DEFAULT_GAIN, self.gain_lag),
        );
        let fade = b.push_fade_gain(path);
        let level = b.push_unit(UnitSpec {
            name: "BinaryOpUGen".to_string(),
            rate: Rate::Control,
            inputs: vec![InputRef::Param(gain), InputRef::Param(fade)],
            num_outputs: 1,
            special_index: 2,
        });
        let level = InputRef::Unit {
            unit: level,
            output: 0,
        };
        // Each written channel: ch * level (BinaryOpUGen multiply, special_index 2).
        // A control-rate channel is lifted to audio first (`Out.ar` reads its
        // inputs strictly as audio - a kr wire would be silence, and multiplying
        // without the K2A ramp would zipper).
        let gained = |b: &mut DspBuilder, ch: InputRef| {
            let ch = b.ensure_audio(ch);
            let unit = b.push_unit(UnitSpec {
                name: "BinaryOpUGen".to_string(),
                rate: Rate::Audio,
                inputs: vec![ch, level],
                num_outputs: 1,
                special_index: 2,
            });
            InputRef::Unit { unit, output: 0 }
        };
        // `Out.ar(0, sigs)`: bus index followed by one signal input per device
        // channel written. A mono input fans across every device channel; a wider
        // input writes channel `i` to bus `i` for the first `min(width,
        // out_channels)` channels - excess input channels are *dropped* (not
        // summed or wrapped), a deficit leaves the upper device channels silent,
        // and only written channels get a gain multiply (dead units would pollute
        // the structural sig). Richer wrap/sum/pan mixing is a later refinement.
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
    // A short de-click lag on the master gain (per "lag the gain, not the freq").
    Out::DEFAULT_GAIN_LAG
}

fn is_default_gain_lag(gain_lag: &f32) -> bool {
    *gain_lag == default_gain_lag()
}
