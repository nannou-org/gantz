//! `~sample`'s egui implementation.

use crate::node::Sample;
use gantz_egui::{
    Env, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind,
};
use std::borrow::Cow;

/// A request to load a WAV file into the `~sample` node at `path`. The node's
/// inspector emits it. The audio driver shows a file dialog, stores the
/// decoded audio as an asset and assigns it to the node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadSample {
    /// The node's path within the head's graph.
    pub path: Vec<gantz_core::node::Id>,
}

impl NodeUi for Sample {
    fn name(&self, _: &Env<'_>) -> Cow<'_, str> {
        Cow::Borrowed("~sample")
    }

    fn description(&self) -> Option<&'static str> {
        Some("An audio sample as a read-only buffer, for example for `~playbuf`")
    }

    fn ui(&mut self, _ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let framed =
            uictx.framed(|ui, _sockets| ui.add(egui::Label::new("~sample").selectable(false)));
        NodeUiResponse::new(framed)
    }

    fn inspector_rows(
        &mut self,
        ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        let summary = match self.asset() {
            Some(asset) => {
                let hex = asset.to_string();
                format!(
                    "{}… {} ch, {} frames, {} Hz",
                    &hex[..hex.len().min(8)],
                    self.channels(),
                    self.frames(),
                    self.sample_rate(),
                )
            }
            None => "none".to_string(),
        };
        let row_h = gantz_egui::widget::node_inspector::table_row_h(body.ui_mut());
        let mut load = false;
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("asset");
            });
            row.col(|ui| {
                ui.horizontal(|ui| {
                    load = ui
                        .button("Load…")
                        .on_hover_text("load a WAV file into this sample")
                        .clicked();
                    ui.label(summary);
                });
            });
        });
        // The load is asynchronous. The driver edits and commits the node
        // when the file is decoded, so this is not a change yet.
        let mut resp = InspectorRowsResponse::default();
        if load {
            resp.emit(LoadSample {
                path: ctx.path().to_vec(),
            });
        }
        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        match kind {
            SocketKind::Input => None,
            SocketKind::Output => {
                Some(SocketDoc::ty("buffer").with_description("the sample, for a buffer socket"))
            }
        }
    }
}
