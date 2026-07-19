use crate::{Env, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind, node, ui_tree::UiTree};

impl NodeUi for gantz_std::Bang {
    fn name(&self, _: &Env<'_>) -> std::borrow::Cow<'_, str> {
        "!".into()
    }

    fn description(&self) -> Option<&'static str> {
        Some("Trigger downstream evaluation")
    }

    fn ui(&mut self, mut ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        // A bang only triggers downstream evaluation; it never edits the
        // node's content address, so the button pushes but never `changed`.
        let (&id, prefix) = ctx.path().split_last().expect("a node path is never empty");
        let tree = fragment(id);
        let root_id = uictx.egui_id().with("gui");
        let mut payloads = Vec::new();
        let framed = uictx.framed(|ui, _sockets| {
            let r = UiTree::new(root_id)
                .instance_prefix(prefix)
                .n_outputs(&|_: &[node::Id]| Some(1))
                .show(&tree, &mut ctx, ui);
            payloads = r.payloads;
            r.inner.unwrap_or_else(|| ui.response())
        });
        let mut resp = NodeUiResponse::new(framed);
        resp.payloads.extend(payloads);
        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        match kind {
            SocketKind::Output => Some(
                SocketDoc::ty("bang")
                    .with_description("empty list '() emitted to trigger downstream evaluation"),
            ),
            SocketKind::Input => {
                Some(SocketDoc::ty("trigger").with_description("ignored; emits a bang when pushed"))
            }
        }
    }
}

/// The bang's button fragment, bound to its own id. The padded label is the
/// bang's visual, not an interpreter default.
fn fragment(id: node::Id) -> gantz_ui::Element {
    gantz_ui::Element::Button(gantz_ui::Button {
        bind: Some(gantz_ui::BindPath(vec![id])),
        label: Some(" ! ".to_string()),
        key: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragment_bakes_bind_and_label() {
        let expected = gantz_ui::Element::Button(gantz_ui::Button {
            bind: Some(gantz_ui::BindPath(vec![5])),
            label: Some(" ! ".to_string()),
            key: None,
        });
        assert_eq!(fragment(5), expected);
    }
}
