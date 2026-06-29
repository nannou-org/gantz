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

/// The fixed width (px) for the optional-numeric dialers in `range`/`prec.`
/// inspector rows, so a following column stays put as a value's width changes.
pub const DIAL_W: f32 = 44.0;

/// Render an optional numeric bound as a tight `<checkbox> <dialer>` group
/// (a [`CheckboxEnabled`](crate::widget::CheckboxEnabled) dialer), suitable as
/// one column of a `range` grid row. The dialer is always shown but disabled
/// while the checkbox is off. `kind` names the bound for the hover text (e.g.
/// `"minimum"`). Returns whether `bound` changed this frame.
pub fn bound_col<T: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    kind: &str,
    bound: &mut Option<T>,
) -> bool {
    let mut on = bound.is_some();
    let mut v = bound.unwrap_or(T::from_f64(0.0));
    let dialer = egui::DragValue::new(&mut v).speed(0.1);
    let resp = ui
        .add(crate::widget::CheckboxEnabled::new(&mut on, dialer).width(DIAL_W))
        .on_hover_text(format!("clamp the {kind} value"));
    if resp.changed() {
        // `on == false` -> None; a just-toggled-on bound takes the default `v`.
        *bound = on.then_some(v);
        return true;
    }
    false
}

/// Render `text` as a label-styled radio option for a mode/style row: dim when
/// unselected, strong when selected (no fill, like the app's tabs). Lay several
/// out in a `ui.horizontal` to form a selector. Returns whether it was just
/// selected.
pub fn radio_option<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    current: &mut T,
    value: T,
    text: &str,
    hover: &str,
) -> bool {
    let strong = ui.visuals().strong_text_color();
    let mut selected = *current == value;
    let resp = ui
        .add(crate::widget::LabelToggle::new(text, &mut selected).selected_color(strong))
        .on_hover_text(hover);
    // Clicking an already-selected option is a no-op (it stays selected).
    if resp.changed() && selected {
        *current = value;
        true
    } else {
        false
    }
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
    // A node may opt out of the default state row (e.g. to summarise a large
    // buffer in its `inspector_ui` instead).
    let state_value = if is_stateful && node.show_state() {
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

/// Working state for the [`socket_doc_rows`] editor, persisted in egui memory so
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

/// Append `type` and `desc.` editor rows for an inlet/outlet marker's stored
/// `ty`/`description` fields (a type label and a note) to the inspector table
/// `body`.
///
/// `type` is a short single-line field; `desc.` is multiline and word-wraps,
/// with its row sized to fit the wrapped text. Edits are buffered in egui memory
/// and written back into `ty`/`description` (trimmed) only on commit (focus loss,
/// or Enter - in the description, Cmd/Ctrl+Enter inserts a newline). Buffering
/// avoids re-seeding (and trimming trailing whitespace) on every keystroke, and
/// means the node - and thus the working graph - only changes on commit, so
/// editing produces a single graph edit rather than one per keystroke. `id_salt`
/// scopes the edit state to the node. Returns whether a commit actually changed
/// `ty`/`description` (so the caller can report the CA-affecting edit), `true`
/// only on the flush frame that writes a new value.
pub(crate) fn socket_doc_rows(
    body: &mut egui_extras::TableBody,
    id_salt: impl std::hash::Hash,
    ty: &mut String,
    description: &mut String,
) -> bool {
    let id = egui::Id::new("socket-doc-editor").with(&id_salt);
    let mut st: SocketDocEditState = body
        .ui_mut()
        .memory(|m| m.data.get_temp(id))
        .unwrap_or_default();

    // Re-seed the buffer only when the stored fields changed externally (never
    // mid-edit, since our own edits aren't written back until committed).
    let cur = (ty.clone(), description.clone());
    if st.seeded != cur {
        st.ty = cur.0.clone();
        st.desc = cur.1.clone();
        st.seeded = cur;
    }

    // `type` is a short single-line label at the default row height.
    let row_h = table_row_h(body.ui_mut());
    let mut ty_resp = None;
    body.row(row_h, |mut row| {
        row.col(|ui| {
            ui.label("type");
        });
        row.col(|ui| {
            ty_resp = Some(
                ui.add(
                    egui::TextEdit::singleline(&mut st.ty)
                        .id(id.with("ty"))
                        .hint_text("type")
                        .desired_width(f32::INFINITY),
                ),
            );
        });
    });

    // `desc.` is multiline and word-wraps; the value column is the table's
    // remainder, so estimate its width conservatively to wrap within the cell
    // and size the row to the wrapped text.
    let wrap_w = (body.ui_mut().available_width() - 64.0).max(64.0);
    let desc_h = doc_field_height(body.ui_mut(), &st.desc, wrap_w);
    let mut desc_resp = None;
    body.row(desc_h, |mut row| {
        row.col(|ui| {
            ui.label("desc.");
        });
        row.col(|ui| {
            // Plain Enter commits: the return key is Cmd/Ctrl+Enter, so a bare
            // Enter surrenders focus instead of inserting a newline.
            desc_resp = Some(
                ui.add(
                    egui::TextEdit::multiline(&mut st.desc)
                        .id(id.with("desc"))
                        .hint_text("description")
                        .desired_rows(1)
                        .desired_width(wrap_w)
                        .return_key(egui::KeyboardShortcut::new(
                            egui::Modifiers::COMMAND,
                            egui::Key::Enter,
                        )),
                ),
            );
        });
    });

    let ty_resp = ty_resp.expect("value column always rendered");
    let desc_resp = desc_resp.expect("value column always rendered");
    // The single-line `type` commits on Enter via focus loss. For the multiline
    // `desc.`, a plain Enter surrenders focus (see the field above) to commit.
    let desc_enter = desc_resp.has_focus()
        && body
            .ui_mut()
            .input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.any());
    if desc_enter {
        desc_resp.surrender_focus();
    }
    let commit = ty_resp.lost_focus() || desc_resp.lost_focus() || desc_enter;

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

    body.ui_mut().memory_mut(|m| m.data.insert_temp(id, st));
    changed
}

/// A table-row height that fits `text` wrapped at `wrap_width` (at least one
/// line), including the multiline `TextEdit`'s vertical margin.
fn doc_field_height(ui: &egui::Ui, text: &str, wrap_width: f32) -> f32 {
    let font_id = egui::TextStyle::Body.resolve(ui.style());
    let line_h = ui.text_style_height(&egui::TextStyle::Body);
    let color = ui.visuals().text_color();
    // Lay out a touch narrower than the field so the measured line count is
    // never below the field's (which would clip); the small extra height is
    // harmless.
    let galley = ui
        .painter()
        .layout(text.to_owned(), font_id, color, (wrap_width - 8.0).max(1.0));
    galley.size().y.max(line_h) + 10.0
}
