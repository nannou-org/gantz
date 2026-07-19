//! The `~scopeout` node: monitor a dsp signal into per-channel ring buffers, read
//! out on a trigger.

use gantz_core::node::{Conns, EvalConf, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_core::steel::SteelVal;
use gantz_nodetag::NodeTag;
use plyphon::Rate;
use plyphon::synthdef::{InputRef, UnitSpec};
use serde::{Deserialize, Serialize};

use crate::dsp::{DspBuilder, NodeDsp, Signal, ToNodeDsp, input_or_silent};

/// A signal *tap*: streams every sample of its input signal into per-channel
/// ring buffers held in VM state (the audio driver writes them, draining a
/// plyphon `ScopeOut` scope stream), and - only on a control-trigger push -
/// outputs the per-channel rings (output 0) and the channel count (output 1).
///
/// The channel count is *inferred* from the input signal's width at synthdef
/// derivation - tap a 2-channel signal and the state carries two rings (the
/// count output reads 0 until the driver first writes). Wire the signal into
/// the dsp input, drive the trigger input (e.g. with a `tick!`), and plug
/// output 0 into a `plot` for a stacked per-channel view. `size` is each ring's
/// length in *frames*; set it to 1 to monitor the latest frame. It is a dsp
/// *sink* (no passthrough); to keep hearing a signal, also wire it to `~out`.
/// A `~peak`/`~rms` node placed before a `~scopeout` gives level metering.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, NodeTag)]
pub struct ScopeOut {
    #[serde(default = "default_size", skip_serializing_if = "is_default_size")]
    size: usize,
}

impl ScopeOut {
    /// The default ring-buffer length (frames) a fresh `~scopeout` starts at.
    pub const DEFAULT_SIZE: usize = 256;

    /// The ring-buffer length in frames.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Set the ring-buffer length in frames (content-address affecting).
    pub fn set_size(&mut self, size: usize) {
        self.size = size.max(1);
    }
}

impl Default for ScopeOut {
    fn default() -> Self {
        ScopeOut {
            size: default_size(),
        }
    }
}

impl gantz_core::Node for ScopeOut {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        // Input 0 is the signal (a dsp edge, any channel width); input 1 is the
        // control trigger that reads the buffers out the outlets.
        2
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        // Output 0 = the per-channel sample rings; output 1 = the channel count.
        2
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        true
    }

    fn branches(&self, _ctx: MetaCtx) -> Vec<EvalConf> {
        // Fire the outlets only when the control trigger is active: branch 0 activates
        // both outputs, branch 1 activates neither. Which branch the `expr` selects at
        // eval time gates whether downstream (e.g. a `plot`) evaluates - so a push
        // arriving through an inert dsp edge no longer surfaces the buffer.
        vec![
            EvalConf::Set(Conns::try_from([true, true]).unwrap()),
            EvalConf::Set(Conns::try_from([false, false]).unwrap()),
        ]
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        let path = ctx.path();
        gantz_core::node::state::init_value_if_absent(ctx.vm(), path, || {
            SteelVal::ListV(Default::default())
        })
        .unwrap()
    }

    fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult {
        // The per-channel ring buffers (maintained by the audio driver) ARE output
        // 0; the channel count - the number of rings, 0 until the driver first
        // writes - is output 1. The trigger is input 1; emit only when it fired
        // this eval - `branches` then gates the outlets. The dsp input (index 0)
        // is inert here (the driver fills the rings), so it is ignored.
        let triggered = ctx.inputs().get(1).is_some_and(Option::is_some);
        let src = if triggered {
            // Branch 0: both outputs active -> `(list rings channel-count)`.
            "(list 0 (list state (length state)))"
        } else {
            // Branch 1: no outputs active.
            "(list 1 '())"
        };
        gantz_core::node::parse_expr(src)
    }
}

impl NodeDsp for ScopeOut {
    fn n_dsp_inputs(&self) -> usize {
        1
    }

    fn n_dsp_outputs(&self) -> usize {
        // A tap sink: it reads the signal, it does not pass it through.
        0
    }

    fn is_monitor(&self) -> bool {
        true
    }

    fn ugens(&self, path: &[usize], inputs: &[Option<Signal>], b: &mut DspBuilder) -> Vec<Signal> {
        // `ScopeOut.ar(bufnum, ch0, ch1, …)`: stream *every* sample of each of the
        // input signal's channels (interleaved) off the audio thread into a cued
        // scope stream the driver drains into this node's per-channel rings - the
        // channel count is the input's width. `bufnum` is a no-lag control param;
        // the driver allocates a globally-unique cued index and sets it via
        // `set_control` after spawning (no def mutation).
        let signal = input_or_silent(inputs, 0);
        let bufnum = b.push_control_param(path, "bufnum");
        let mut scope_inputs = Vec::with_capacity(signal.width() + 1);
        scope_inputs.push(InputRef::Param(bufnum));
        scope_inputs.extend(signal.channels());
        let scope_unit = b.push_unit(UnitSpec::new("ScopeOut", Rate::Audio, scope_inputs, 0));
        b.push_monitor(
            path,
            self.size,
            signal.width(),
            scope_unit as usize,
            bufnum as usize,
        );
        vec![]
    }
}

impl ToNodeDsp for ScopeOut {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        Some(self)
    }
}

fn default_size() -> usize {
    ScopeOut::DEFAULT_SIZE
}

fn is_default_size(size: &usize) -> bool {
    *size == default_size()
}
