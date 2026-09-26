//! [`UnitNode`]'s egui implementation, driven by its descriptor row.

use crate::dsp::BufferAccess;
use crate::egui::param::{param_row, params_state_row, rate_row, value_row};
use crate::node::UnitNode;
use crate::param::{param_value_keyed, params_state, with_param_value};
use crate::units::{Emit, In, UnitRate};
use gantz_egui::{
    Env, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind,
};

impl NodeUi for UnitNode {
    fn name(&self, _: &Env<'_>) -> std::borrow::Cow<'_, str> {
        self.desc().keyword.into()
    }

    fn description(&self) -> Option<&'static str> {
        Some(self.desc().doc)
    }

    fn ui(&mut self, _ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        // The body shows only the node name. Params are edited in the inspector.
        let keyword = self.desc().keyword;
        let framed =
            uictx.framed(|ui, _sockets| ui.add(egui::Label::new(keyword).selectable(false)));
        NodeUiResponse::new(framed)
    }

    fn show_state(&self) -> bool {
        // The summarised "N queued" row in `inspector_rows` replaces the raw
        // keyed state dump.
        false
    }

    fn inspector_rows(
        &mut self,
        ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        let mut resp = InspectorRowsResponse::default();
        let desc = self.desc();
        if desc.hybrid_params().next().is_some() {
            let state = ctx.extract_value().ok().flatten();
            params_state_row(body, state.as_ref());
        }
        for entry in desc.inputs {
            match entry {
                // Param values live in keyed VM state. A value edit must never
                // change the content address. Lags live in the weight. A lag
                // edit is structural.
                In::Param {
                    name,
                    default,
                    min,
                    max,
                    suffix,
                    ..
                } => {
                    // Re-extracted per row so one row's write is never
                    // clobbered by a stale snapshot in another.
                    let state = ctx.extract_value().ok().flatten();
                    let mut value = state
                        .as_ref()
                        .and_then(|s| param_value_keyed(s, name))
                        .unwrap_or(*default as f64) as f32;
                    let mut lag = self.lag(name);
                    let dv = egui::DragValue::new(&mut value)
                        .range(*min..=*max)
                        .speed(((max - min) as f64 / 2_000.0).max(0.000_5))
                        .suffix(*suffix);
                    let (value_changed, lag_changed) = param_row(body, name, dv, &mut lag);
                    if value_changed {
                        // Preserve the other params and any queued `pending`
                        // updates. Only this value changes.
                        let prev = state.unwrap_or_else(|| {
                            let defaults: Vec<(&str, f64)> =
                                desc.hybrid_params().map(|(n, d)| (n, d as f64)).collect();
                            params_state(&defaults)
                        });
                        let _ = ctx.update_value(with_param_value(prev, name, value as f64));
                    }
                    if lag_changed {
                        self.set_lag(name, lag);
                        resp.mark_changed();
                    }
                }
                // Init-only values are baked into the def as constants, so an
                // edit is structural and re-derives.
                In::Init { name, .. } => {
                    let mut value = self.init_value(name);
                    let dv = egui::DragValue::new(&mut value).speed(0.001);
                    if value_row(body, name, dv) {
                        self.set_init(name, value);
                        resp.mark_changed();
                    }
                }
                In::Signal { .. } | In::Buffer { .. } | In::Group { .. } | In::Baked(_) => (),
            }
        }
        // The channel count sets the outputs, so an edit is structural.
        if let Emit::InitChannels { name, max, .. } = desc.emit {
            let mut value = self.init_value(name);
            let dv = egui::DragValue::new(&mut value)
                .range(1.0..=max as f32)
                .speed(0.05)
                .max_decimals(0);
            if value_row(body, name, dv) {
                self.set_init(name, value.round());
                resp.mark_changed();
            }
        }
        // A fixed-rate row has no rate to choose.
        if desc.rate == UnitRate::Any {
            let mut rate = self.rate();
            if rate_row(body, &mut rate) {
                self.set_rate(rate);
                resp.mark_changed();
            }
        }
        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, ix: usize) -> Option<SocketDoc> {
        let desc = self.desc();
        match kind {
            SocketKind::Input => {
                let socket = desc.sockets().nth(ix)?;
                match socket {
                    In::Signal { doc, .. } => Some(SocketDoc::ty("signal").with_description(*doc)),
                    In::Param { doc, .. } => {
                        Some(SocketDoc::ty("signal | number").with_description(format!(
                            "{doc} - a connected signal drives it directly, a connected \
                             number overrides the inspector value"
                        )))
                    }
                    In::Buffer { doc, access, .. } => {
                        let note = match access {
                            BufferAccess::Read => "a `~sample` or `~buffer`",
                            BufferAccess::Write => "a `~buffer`. The unit writes it",
                        };
                        Some(SocketDoc::ty("buffer").with_description(format!("{doc} - {note}")))
                    }
                    In::Group { doc, .. } => Some(SocketDoc::ty("signal").with_description(*doc)),
                    In::Baked(_) | In::Init { .. } => None,
                }
            }
            SocketKind::Output => desc
                .outputs
                .get(ix)
                .map(|doc| SocketDoc::ty("signal").with_description(*doc)),
        }
    }
}
