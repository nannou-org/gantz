use crate::{Env, EvalEntry, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind};
use gantz_io::MainBang;
use gantz_nodetag::NodeTag;
use petgraph::visit::{IntoNodeReferences, NodeRef};

impl NodeUi for MainBang {
    fn name(&self, _: &Env<'_>) -> std::borrow::Cow<'_, str> {
        "main!".into()
    }

    fn description(&self) -> Option<&'static str> {
        Some(
            "Fires when the graph runs as a program, for example with `gantz run`. \
             Every main! at the graph's root fires together. Outputs the program \
             arguments, which are empty in the app. Double-click to fire.",
        )
    }

    fn ui(&mut self, ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let framed =
            uictx.framed(|ui, _sockets| ui.add(egui::Label::new("main!").selectable(false)));
        let mut resp = NodeUiResponse::new(framed);
        if resp.framed.inner.response.double_clicked() {
            let ids = ctx
                .graph()
                .node_references()
                .filter(|n| n.weight().tag == MainBang::TAG)
                .map(|n| n.id().index());
            if let Some(ep) = gantz_io::main_bang::entrypoint_of(ids) {
                resp.emit(EvalEntry(ep));
            }
        }
        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        match kind {
            SocketKind::Output => Some(
                SocketDoc::ty("list")
                    .with_description("the program arguments as strings, '() in the app"),
            ),
            SocketKind::Input => None,
        }
    }
}
