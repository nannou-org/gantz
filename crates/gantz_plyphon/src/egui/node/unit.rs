//! [`UnitNode`]'s egui implementation, driven by its descriptor row.

use crate::egui::param::{param_row, params_state_row, rate_row, value_row};
use crate::node::UnitNode;
use crate::param::{param_value_keyed, params_state, with_param_value};
use crate::units::In;
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
        // The body shows just the node name; params are edited in the inspector.
        let keyword = self.desc().keyword;
        let framed =
            uictx.framed(|ui, _sockets| ui.add(egui::Label::new(keyword).selectable(false)));
        NodeUiResponse::new(framed)
    }

    fn show_state(&self) -> bool {
        // A summarised "N queued" state row (in `inspector_rows`) replaces the
        // raw keyed state dump.
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
                // Param values live in keyed VM state (a value edit must NOT
                // change the content address); lags live in the weight (a lag
                // edit is structural).
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
                        // updates; only this value changes.
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
                // Init-only values are structural: they are baked into the
                // def as constants, so an edit re-derives (respawns).
                In::Init { name, .. } => {
                    let mut value = self.init_value(name);
                    let dv = egui::DragValue::new(&mut value).speed(0.001);
                    if value_row(body, name, dv) {
                        self.set_init(name, value);
                        resp.mark_changed();
                    }
                }
                In::Signal { .. } | In::Baked(_) => (),
            }
        }
        let mut rate = self.rate();
        if rate_row(body, &mut rate) {
            self.set_rate(rate);
            resp.mark_changed();
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
