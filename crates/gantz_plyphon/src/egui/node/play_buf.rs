//! `~playbuf`'s egui implementation.

use crate::node::PlayBuf;
use gantz_egui::{
    Env, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind,
};
use std::borrow::Cow;

impl NodeUi for PlayBuf {
    fn name(&self, _: &Env<'_>) -> Cow<'_, str> {
        Cow::Borrowed("~playbuf")
    }

    fn description(&self) -> Option<&'static str> {
        Some("Play a sample buffer back (looping), one output per buffer channel")
    }

    fn ui(&mut self, _ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let framed =
            uictx.framed(|ui, _sockets| ui.add(egui::Label::new("~playbuf").selectable(false)));
        NodeUiResponse::new(framed)
    }

    fn inspector_rows(
        &mut self,
        _ctx: &mut NodeCtx,
        _body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        // Assigning/browsing assets from the UI is a follow-up; for now the asset
        // is set programmatically and the node has no editable rows.
        InspectorRowsResponse::default()
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        match kind {
            SocketKind::Input => None,
            SocketKind::Output => Some(
                SocketDoc::ty("signal")
                    .with_description("the buffer's samples (one channel per buffer channel)"),
            ),
        }
    }
}
