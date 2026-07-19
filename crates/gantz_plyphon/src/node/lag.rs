//! The `~lag` node: a one-pole smoother over a configurable duration.

use gantz_core::node::{ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_nodetag::NodeTag;
use plyphon::synthdef::{InputRef, UnitSpec};
use serde::{Deserialize, Serialize};

use crate::dsp::{DspBuilder, NodeDsp, NodeRate, Signal, ToNodeDsp, input_or_silent};
use crate::param::{param_name, param_state, plyphon_param};

/// A one-pole lag (smoother). Emits a `Lag` UGen per input channel at the
/// configured `rate`, smoothing the whole signal group over the shared `lag`
/// duration. (A `kr` lag fed by an audio-rate wire smooths the block's first
/// sample - the usual audio-to-control collapse.)
///
/// The `lag` duration (seconds) lives in the node's VM state (like `~sinosc`'s
/// freq), edited via the inspector and applied to the running synth via
/// `set_control` (the `Lag` UGen recomputes its coefficient on change, so it is
/// click-free). Only the (structural) `rate` lives in the node weight.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash, NodeTag)]
pub struct Lag {
    #[serde(default, skip_serializing_if = "crate::node::is_default")]
    rate: NodeRate,
}

impl Lag {
    /// The default lag time (seconds) a fresh `~lag` starts at.
    pub const DEFAULT_DUR: f32 = 0.1;

    /// The ugen rate (`ar`/`kr`) the smoother runs at.
    pub fn rate(&self) -> NodeRate {
        self.rate
    }

    /// Set the ugen rate (content-address affecting; structural).
    pub fn set_rate(&mut self, rate: NodeRate) {
        self.rate = rate;
    }
}

impl gantz_core::Node for Lag {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        // A single dsp signal input (the duration lives in state, not a socket).
        1
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        true
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        let path = ctx.path();
        gantz_core::node::state::init_value_if_absent(ctx.vm(), path, || {
            param_state(Self::DEFAULT_DUR as f64)
        })
        .unwrap()
    }

    fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
        // Steel-inert: the audio engine smooths the signal; the duration lives in
        // state and is applied via `set_control`. A non-numeric placeholder
        // output feeds the inert dsp output edge (see the `NodeDsp` docs).
        gantz_core::node::parse_expr("'()")
    }
}

impl NodeDsp for Lag {
    fn n_dsp_inputs(&self) -> usize {
        1
    }

    fn n_dsp_outputs(&self) -> usize {
        1
    }

    fn ugens(&self, path: &[usize], inputs: &[Option<Signal>], b: &mut DspBuilder) -> Vec<Signal> {
        let signal = input_or_silent(inputs, 0);
        // The lag time is a settable control param (nominal default here; the
        // driver applies the live state value via `set_control`), shared by every
        // channel's `Lag` unit (params broadcast across the group).
        let dur = b.push_param(
            path,
            plyphon_param(param_name(path, "dur"), Self::DEFAULT_DUR, 0.0),
        );
        // One `Lag` per channel: width N in -> width N out.
        let smoothed = signal
            .channels()
            .map(|ch| {
                let unit = b.push_unit(UnitSpec::new(
                    "Lag",
                    self.rate.to_plyphon(),
                    vec![ch, InputRef::Param(dur)],
                    1,
                ));
                InputRef::Unit { unit, output: 0 }
            })
            .collect();
        vec![smoothed]
    }
}

impl ToNodeDsp for Lag {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        Some(self)
    }
}
