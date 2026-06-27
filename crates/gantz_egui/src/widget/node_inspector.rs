use crate::{InspectorRowsResponse, NodeCtx, NodeUi, response::DynResponse};
use egui::scroll_area::ScrollAreaOutput;
use egui_extras::{Column, TableBuilder};
use gantz_core::node::{self, MetaCtx, Node};

/// A widget for presenting more detailed information and control for a node.
pub struct NodeInspector<'a, N> {
    node: &'a mut N,
    ctx: NodeCtx<'a>,
    immutable: bool,
}

/// The response returned from [`NodeInspector::show`].
pub struct NodeInspectorResponse {
    pub scroll_area_output: ScrollAreaOutput<()>,
    pub node_response: Option<egui::Response>,
    pub label_response: egui::Response,
    /// Whether the inspector made a CA-affecting edit to the node this frame
    /// (from its rows or its extra UI). See [`NodeUi`].
    pub changed: bool,
    /// Payloads emitted from the inspector UI, for the application to handle.
    pub payloads: Vec<DynResponse>,
}

impl<'a, N> NodeInspector<'a, N>
where
    N: Node + NodeUi,
{
    pub fn new(node: &'a mut N, ctx: NodeCtx<'a>, immutable: bool) -> Self {
        Self {
            node,
            ctx,
            immutable,
        }
    }

    pub fn show(self, ui: &mut egui::Ui) -> NodeInspectorResponse {
        let Self {
            node,
            mut ctx,
            immutable,
        } = self;
        let (scroll_area_output, label_response, rows) = table(node, &mut ctx, immutable, ui);
        if immutable {
            ui.disable();
        }
        let extra = node.inspector_ui(ctx, ui);
        let mut payloads = rows.payloads;
        payloads.extend(extra.payloads);
        NodeInspectorResponse {
            scroll_area_output,
            node_response: extra.inner,
            label_response,
            changed: rows.changed || extra.changed,
            payloads,
        }
    }
}

pub fn table_row_h(ui: &egui::Ui) -> f32 {
    ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y
}

pub fn table(
    node: &mut (impl Node + NodeUi),
    ctx: &mut NodeCtx,
    immutable: bool,
    ui: &mut egui::Ui,
) -> (ScrollAreaOutput<()>, egui::Response, InspectorRowsResponse) {
    // Extract info we need upfront before the closure borrows ctx.
    let registry = ctx.registry();
    let get_node = |ca: &gantz_ca::ContentAddr| registry.node(ca);
    let meta_ctx = MetaCtx::new(&get_node);

    // Compute all node metadata before the table closure.
    let name = node.name(registry);
    let path = ctx.path().to_vec();
    let n_inputs = node.n_inputs(meta_ctx);
    let n_outputs = node.n_outputs(meta_ctx);
    let push_eval = !node.push_eval(meta_ctx).is_empty();
    let pull_eval = !node.pull_eval(meta_ctx).is_empty();
    let is_stateful = node.stateful(meta_ctx);
    let state_value = if is_stateful {
        Some(ctx.extract_value())
    } else {
        None
    };

    let label_response = ui.add(
        egui::Label::new(egui::RichText::new(name).strong())
            .selectable(false)
            .sense(egui::Sense::click()),
    );
    ui.add_space(ui.spacing().item_spacing.y);
    let row_h = table_row_h(ui);
    let mut rows_resp = InspectorRowsResponse::default();
    let scroll_area_output = TableBuilder::new(ui)
        .vscroll(false)
        .column(Column::auto().at_least(50.0).resizable(true))
        .column(Column::remainder().at_least(120.0))
        .body(|mut body| {
            body.row(row_h, |mut row| {
                row.col(|ui| {
                    ui.label("path");
                });
                row.col(|ui| {
                    ui.monospace(path_string(&path));
                });
            });

            body.row(row_h, |mut row| {
                row.col(|ui| {
                    ui.label("i/o");
                });
                row.col(|ui| {
                    ui.label(format!("{} inputs, {} outputs", n_inputs, n_outputs));
                });
            });

            let eval = match (push_eval, pull_eval) {
                (true, true) => Some("push, pull"),
                (true, false) => Some("push"),
                (false, true) => Some("pull"),
                (false, false) => None,
            };

            if let Some(eval) = eval {
                body.row(row_h, |mut row| {
                    row.col(|ui| {
                        ui.label("eval");
                    });
                    row.col(|ui| {
                        ui.label(eval);
                    });
                });
            }

            if let Some(ref state_result) = state_value {
                body.row(row_h, |mut row| {
                    row.col(|ui| {
                        ui.label("state");
                    });
                    row.col(|ui| match state_result {
                        Ok(Some(state)) => {
                            ui.label(format!("{state:#?}"));
                        }
                        Ok(None) => {
                            ui.weak("None");
                        }
                        Err(_) => {
                            ui.weak("Error");
                        }
                    });
                });
            }

            if immutable {
                body.ui_mut().disable();
            }
            rows_resp = node.inspector_rows(ctx, &mut body);
        });
    (scroll_area_output, label_response, rows_resp)
}

/// Format the node's path string.
pub fn path_string(path: &[node::Id]) -> String {
    path.iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Working state for the [`socket_doc_editor`], persisted in egui memory so
/// in-progress text isn't clobbered by re-seeding every frame.
#[derive(Clone, Default)]
struct SocketDocEditState {
    ty: String,
    desc: String,
    /// The `(ty, desc)` of the `current` doc we last seeded from, used to detect
    /// external changes (e.g. a carried-forward edit) without overwriting
    /// in-progress typing.
    seeded: (String, String),
}

/// A small editor for an inlet/outlet marker's stored `ty`/`description`
/// fields (a type label and a longer note).
///
/// Edits are buffered in egui memory and written back into `ty`/`description`
/// (trimmed) only when a field loses focus or the user presses Enter in the
/// description (Ctrl/Cmd+Enter inserts a newline). Buffering avoids re-seeding
/// (and trimming trailing whitespace) on every keystroke, and means the node -
/// and thus the working graph - only changes on commit, so editing produces a
/// single graph edit rather than one per keystroke. `id_salt` scopes the edit
/// state to the node. Returns the combined field response together with whether
/// a commit actually changed `ty`/`description` (so the caller can report the
/// CA-affecting edit), `true` only on the flush frame that writes a new value.
pub(crate) fn socket_doc_editor(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    ty: &mut String,
    description: &mut String,
) -> (egui::Response, bool) {
    let id = egui::Id::new("socket-doc-editor").with(&id_salt);
    let mut st: SocketDocEditState = ui.memory(|m| m.data.get_temp(id)).unwrap_or_default();

    // Re-seed the buffer only when the stored fields changed externally (never
    // mid-edit, since our own edits aren't written back until committed).
    let cur = (ty.clone(), description.clone());
    if st.seeded != cur {
        st.ty = cur.0.clone();
        st.desc = cur.1.clone();
        st.seeded = cur;
    }

    let ty_resp = ui.add(
        egui::TextEdit::singleline(&mut st.ty)
            .id(id.with("ty"))
            .hint_text("type")
            .desired_width(f32::INFINITY),
    );
    // Plain Enter commits; Ctrl/Cmd+Enter inserts a newline.
    let desc_resp = ui.add(
        egui::TextEdit::multiline(&mut st.desc)
            .id(id.with("desc"))
            .hint_text("description")
            .desired_rows(2)
            .desired_width(f32::INFINITY)
            .return_key(egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::Enter,
            )),
    );
    let desc_enter = desc_resp.has_focus()
        && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.any());
    // Drop focus on commit so the description behaves like the single-line type
    // field (whose default return-key handling already surrenders focus).
    if desc_enter {
        desc_resp.surrender_focus();
    }
    let commit = ty_resp.lost_focus() || desc_resp.lost_focus() || desc_enter;

    let resp = ty_resp.union(desc_resp);

    let mut changed = false;
    if commit {
        let new = (st.ty.trim().to_string(), st.desc.trim().to_string());
        // Only a commit that actually alters the stored fields is a CA-affecting
        // edit; committing unchanged text (e.g. a bare focus loss) is not.
        changed = new.0 != *ty || new.1 != *description;
        *ty = new.0.clone();
        *description = new.1.clone();
        // Keep the seed in sync with what we just wrote back so the trimmed
        // values aren't treated as an external change next frame.
        st.ty = new.0.clone();
        st.desc = new.1.clone();
        st.seeded = new;
    }

    ui.memory_mut(|m| m.data.insert_temp(id, st));
    (resp, changed)
}
