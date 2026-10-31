//! The `~scopeout` node. It monitors a dsp signal into per-channel ring
//! buffers, read out on a trigger.

use gantz_ca::Datum;
use gantz_nodedef::{ExprArgs, Meta, View};
use gantz_nodetag::NodeTag;
use gantz_ui::{self as ui, BindPath, Element};
use plyphon::Rate;
use plyphon::synthdef::{InputRef, UnitSpec};
use serde::{Deserialize, Serialize};

use crate::dsp::{DspBuilder, NodeDsp, Signal, ToNodeDsp, input_or_silent};
use crate::view::{doc, edit_count, edit_dialer, row};

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

/// The edit of the ring length.
const SIZE: &str = "size";

/// The longest ring in frames.
const MAX_SIZE: usize = 16_384;

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

impl gantz_nodedef::NodeDef for ScopeOut {
    const TAG: &'static str = <Self as NodeTag>::TAG;

    fn meta(&self) -> Meta {
        Meta {
            name: "~scopeout".into(),
            description: "Scope a signal into per-channel ring buffers. Read them out on a trigger"
                .into(),
            // Input 0 is the signal, a dsp edge of any channel width. Input 1
            // is the control trigger that reads the buffers out the outlets.
            inputs: 2,
            // Output 0 is the per-channel sample rings. Output 1 is the
            // channel count.
            outputs: 2,
            // Fire the outlets only when the control trigger is active.
            // Branch 0 activates both outputs, branch 1 activates neither. The
            // branch the `expr` selects at eval time gates whether downstream
            // nodes such as a `plot` evaluate. A push arriving through an
            // inert dsp edge therefore never surfaces the buffer.
            branches: [[true, true], [false, false]]
                .map(|set| set.into_iter().collect())
                .into_iter()
                .collect(),
            stateful: true,
            ..Meta::default()
        }
    }

    fn expr(&self, ctx: ExprArgs<'_>) -> Result<String, String> {
        // The per-channel ring buffers the DSP runtime maintains are output
        // 0. The channel count, the number of rings, is output 1. It is 0
        // until the runtime first writes. The trigger is input 1. Emit only
        // when it fired this eval, then `branches` gates the outlets. The dsp
        // input at index 0 is inert here, since the runtime fills the rings,
        // so it is ignored.
        let triggered = ctx.inputs.get(1).is_some_and(Option::is_some);
        let src = match triggered {
            // Branch 0, both outputs active, yields `(list rings channel-count)`.
            true => "(list 0 (list state (length state)))",
            // Branch 1, no outputs active.
            false => "(list 1 '())",
        };
        Ok(src.into())
    }

    fn init_state(&self) -> Option<Datum> {
        Some(Datum::Seq(vec![]))
    }

    fn view(&self) -> View {
        let size = ui::Dialer {
            min: Some(1.0),
            max: Some(MAX_SIZE as f64),
            step: Some(1.0),
            precision: Some(0),
            label: Some("frames".into()),
            tip: Some("the length of each ring, the frames that a read gives".into()),
            ..edit_dialer(SIZE, self.size as f64)
        };
        // The buffered samples of each channel, stacked.
        let rings = ui::Plot {
            bind: Some(BindPath(vec![])),
            h: Some(80.0),
            ..ui::Plot::default()
        };
        let signal = "signal to sample into per-channel ring buffers (any channel width)";
        let rings_doc = "one list of buffered samples per channel (oldest first)";
        let count_doc = "the channel count (0 until the DSP runtime writes)";
        View {
            inputs: vec![
                doc("signal", signal),
                doc("bang", "trigger: output the buffered samples"),
            ]
            .into(),
            outputs: vec![doc("list", rings_doc), doc("number", count_doc)],
            rows: vec![row("size", Element::Dialer(size))],
            inspector: Some(Element::Plot(rings)),
            hide_state: true,
            ..View::default()
        }
    }

    fn edit(&self, edit: &str, value: &Datum) -> Option<Self> {
        let size = edit_count(value, [1, MAX_SIZE]).filter(|_| edit == SIZE)?;
        Some(ScopeOut { size })
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
