//! The `~scopeout` node. It monitors a dsp signal into per-channel ring
//! buffers, read out on a trigger.

use gantz_core::node::{Conns, EvalConf, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_core::steel::SteelVal;
use gantz_nodetag::NodeTag;
use plyphon::Rate;
use plyphon::synthdef::{InputRef, UnitSpec};
use serde::{Deserialize, Serialize};

use crate::dsp::{DspBuilder, NodeDsp, Signal, ToNodeDsp, input_or_silent};

/// A signal tap. It streams every sample of its input signal into
/// per-channel ring buffers held in VM state. The audio driver writes them by
/// draining a plyphon `ScopeOut` scope stream. Only on a control-trigger push
/// does it output the per-channel rings on output 0 and the channel count on
/// output 1.
///
/// The channel count is inferred from the input signal's width at synthdef
/// derivation. Tap a 2-channel signal and the state carries two rings. The
/// count output reads 0 until the driver first writes. Wire the signal into
/// the dsp input, drive the trigger input with a `tick!`, and plug output 0
/// into a `plot` for a stacked per-channel view. `size` is each ring's length
/// in frames. Set it to 1 to monitor the latest frame. It is a dsp sink with
/// no passthrough. To keep hearing a signal, also wire it to `~out`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, NodeTag)]
pub struct ScopeOut {
    #[serde(default = "default_size", skip_serializing_if = "is_default_size")]
    size: usize,
}

impl ScopeOut {
    /// The default ring-buffer length in frames a fresh `~scopeout` starts at.
    pub const DEFAULT_SIZE: usize = 256;

    /// The ring-buffer length in frames.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Set the ring-buffer length in frames. It affects the content address.
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
        // Input 0 is the signal, a dsp edge of any channel width. Input 1 is
        // the control trigger that reads the buffers out the outlets.
        2
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        // Output 0 is the per-channel sample rings. Output 1 is the channel
        // count.
        2
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        true
    }

    fn branches(&self, _ctx: MetaCtx) -> Vec<EvalConf> {
        // Fire the outlets only when the control trigger is active. Branch 0
        // activates both outputs, branch 1 activates neither. The branch the
        // `expr` selects at eval time gates whether downstream nodes such as a
        // `plot` evaluate. A push arriving through an inert dsp edge therefore
        // never surfaces the buffer.
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
        // The per-channel ring buffers the audio driver maintains are output
        // 0. The channel count, the number of rings, is output 1. It is 0
        // until the driver first writes. The trigger is input 1. Emit only
        // when it fired this eval, then `branches` gates the outlets. The dsp
        // input at index 0 is inert here, since the driver fills the rings, so
        // it is ignored.
        let triggered = ctx.inputs().get(1).is_some_and(Option::is_some);
        let src = if triggered {
            // Branch 0, both outputs active, yields `(list rings channel-count)`.
            "(list 0 (list state (length state)))"
        } else {
            // Branch 1, no outputs active.
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
        // A tap sink. It reads the signal and does not pass it through.
        0
    }

    fn is_monitor(&self) -> bool {
        true
    }

    fn ugens(&self, path: &[usize], inputs: &[Option<Signal>], b: &mut DspBuilder) -> Vec<Signal> {
        // `ScopeOut.ar(bufnum, ch0, ch1, ...)` streams every sample of each of
        // the input signal's channels, interleaved, off the audio thread into
        // a cued scope stream. The driver drains it into this node's
        // per-channel rings. The channel count is the input's width. `bufnum`
        // is a no-lag control param. The driver allocates a globally-unique
        // cued index and sets it via `set_control` after spawning, with no def
        // mutation.
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
