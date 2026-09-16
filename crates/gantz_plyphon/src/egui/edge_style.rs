//! The DSP domain's graph-scene edge styling. Signal edges render
//! distinctly from control edges. See [`DspEdgeStyle`].

use std::collections::HashMap;
use std::sync::Arc;

use gantz_egui::widget::{EdgeStyle, EdgeStyleCtx, EdgeStyling};
use plyphon::Rate;

use crate::describe::rate_token;
use crate::dsp::PortShape;
use crate::port_info::RootPortInfo;

/// The DSP domain's [`EdgeStyle`]. An edge from a signal output into a
/// signal input styles by the source port's derive-time shape. It gets one
/// strand per channel, a rate-coded dash and a width and rate hover tooltip.
/// Control edges and non-DSP heads keep the default styling.
///
/// The per-head port classification requires the concrete node type. A
/// provider computes it where that type is known and hands it over here. See
/// [`root_port_info`][crate::root_port_info].
#[derive(Debug, Default)]
pub struct DspEdgeStyle {
    /// Each open head's root port classification.
    pub heads: HashMap<gantz_ca::Head, Arc<RootPortInfo>>,
}

/// The notch spacing in graph units of a signal edge's notched-cord texture.
/// The notches are even dashes overpainted in the extreme background colour.
/// They are the theme-neutral signal cue. Rate stays encoded as the base dash.
const SIGNAL_NOTCH: f32 = 6.0;

/// A signal edge's stroke is heavier than a control edge's. This reinforces
/// the notched cord as a signal.
const SIGNAL_WIDTH_SCALE: f32 = 1.8;

impl EdgeStyle for DspEdgeStyle {
    fn edge_styling(&self, ctx: &EdgeStyleCtx) -> Option<EdgeStyling> {
        let info = self.heads.get(ctx.head)?;
        if !info.signal_inputs.contains(&ctx.dst) {
            return None;
        }
        let shape = info.signal_outputs.get(&ctx.src)?;
        Some(dsp_edge_styling(*shape))
    }
}

/// The styling of a signal edge whose source port recorded `shape` at derive
/// time. `None` means the port is signal-classified but derivation
/// materialized nothing for it. The port feeds no sink, or the head's shapes
/// are unavailable. For example, an inlet or outlet boundary edge in a nested
/// view.
///
/// A notched-cord texture distinguishes signal edges from control edges, not
/// colour. Rate is the base dash pattern. Audio is solid, control is dashed
/// and scalar and demand are dotted. Channel width is the parallel strand
/// count.
fn dsp_edge_styling(shape: Option<PortShape>) -> EdgeStyling {
    let mut styling = EdgeStyling::default();
    styling.width_scale = SIGNAL_WIDTH_SCALE;
    let Some(shape) = shape else {
        // Nothing materialized. Keep the notched cadence but leave the
        // off-segments as gaps rather than filled. This "open" cord reads as
        // an unmaterialised signal.
        styling.dash = Some((SIGNAL_NOTCH, SIGNAL_NOTCH));
        styling.hover_text = Some("signal".to_string());
        return styling;
    };
    styling.notch = Some(SIGNAL_NOTCH);
    styling.strands = shape.width;
    styling.hover_text = Some(format!("{}ch {}", shape.width, rate_token(shape.rate)));
    match shape.rate {
        Rate::Audio => {}
        Rate::Control => styling.dash = Some((6.0, 4.0)),
        // A short dash reads as dots.
        Rate::Scalar | Rate::Demand => styling.dash = Some((1.5, 3.0)),
    }
    styling
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Signal edges style only when the source is a signal output and the
    /// destination is a signal input of the viewed head. They use a
    /// notched-cord texture, not colour, and pass the true channel count. The
    /// painter, not the styler, renders wide bundles.
    #[test]
    fn styles_signal_edges_only() {
        let mut info = RootPortInfo::default();
        info.signal_outputs.insert(
            (0, 0),
            Some(PortShape {
                width: 2,
                rate: Rate::Audio,
            }),
        );
        // A wide bus output, to check the strand count is not clamped here.
        info.signal_outputs.insert(
            (3, 0),
            Some(PortShape {
                width: 8,
                rate: Rate::Audio,
            }),
        );
        // An unmaterialised signal output with no recorded shape.
        info.signal_outputs.insert((4, 0), None);
        info.signal_inputs.insert((1, 0));
        let head = gantz_ca::Head::Branch("test".parse().unwrap());
        let style = DspEdgeStyle {
            heads: [(head.clone(), Arc::new(info))].into_iter().collect(),
        };
        let ctx = |src: (usize, usize), dst: (usize, usize)| EdgeStyleCtx::new(&head, src, dst);

        // The signal edge gets a heavier notched cord, no colour,
        // per-channel strands and the width and rate tooltip.
        let styling = style.edge_styling(&ctx((0, 0), (1, 0))).unwrap();
        assert!(styling.notch.is_some());
        assert!(styling.width_scale > 1.0);
        assert!(styling.color.is_none());
        assert_eq!(styling.strands, 2);
        assert_eq!(styling.hover_text.as_deref(), Some("2ch ar"));
        // The wide bus passes its true channel count and reports it in the
        // tooltip. Abridging is the painter's job.
        let wide = style.edge_styling(&ctx((3, 0), (1, 0))).unwrap();
        assert_eq!(wide.strands, 8);
        assert_eq!(wide.hover_text.as_deref(), Some("8ch ar"));
        // An unmaterialised signal edge is an "open" cord. It has gapped
        // dashes, no notch fill and the "signal" tooltip.
        let open = style.edge_styling(&ctx((4, 0), (1, 0))).unwrap();
        assert!(open.notch.is_none());
        assert!(open.dash.is_some());
        assert_eq!(open.hover_text.as_deref(), Some("signal"));
        // A control destination, the `~out` gain input 1, keeps the default.
        assert!(style.edge_styling(&ctx((0, 0), (1, 1))).is_none());
        // A control source into a signal input keeps the default. That is a
        // hybrid input's control feed.
        assert!(style.edge_styling(&ctx((2, 0), (1, 0))).is_none());
        // An unknown head keeps the default.
        let other = gantz_ca::Head::Branch("other".parse().unwrap());
        let ctx = EdgeStyleCtx::new(&other, (0, 0), (1, 0));
        assert!(style.edge_styling(&ctx).is_none());
    }
}
