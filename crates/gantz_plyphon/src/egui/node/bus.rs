//! `~bus`'s egui implementation.

use crate::node::Bus;
use gantz_egui::{Env, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind};

impl NodeUi for Bus {
    fn name(&self, _: &Env<'_>) -> std::borrow::Cow<'_, str> {
        "~bus".into()
    }

    fn description(&self) -> Option<&'static str> {
        Some("Synthdef boundary: edits beyond it leave this side's synth running")
    }

    fn ui(&mut self, _ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let framed =
            uictx.framed(|ui, _sockets| ui.add(egui::Label::new("~bus").selectable(false)));
        NodeUiResponse::new(framed)
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        match kind {
            SocketKind::Input => Some(SocketDoc::ty("signal").with_description(
                "signal to carry across the synthdef boundary (any channel width)",
            )),
            SocketKind::Output => Some(
                SocketDoc::ty("signal")
                    .with_description("the same signal, entering the downstream synthdef"),
            ),
        }
    }
}
