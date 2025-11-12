//! A simple widget for selecting between, naming and creating new graphs.

use std::collections::{BTreeMap, HashSet};

/// A widget for selecting between, naming, and creating new graphs.
pub struct GraphSelect<'a> {
    id: egui::Id,
    graph_reg: &'a dyn GraphRegistry,
    head: &'a gantz_ca::Head,
}

#[derive(Clone, Default)]
struct GraphSelectState {
    /// The last head provided via argument.
    /// We track this to know if we should reset the working graph name.
    last_head: Option<gantz_ca::Head>,
    working_graph_name: String,
}

/// Methods required on the provided graph registry.
pub trait GraphRegistry {
    /// All selectable commit addresses.
    fn commits(&self) -> Vec<(&gantz_ca::CommitAddr, &gantz_ca::Commit)>;
    /// An iterator yielding all name -> CA pairs.
    fn names(&self) -> &GraphNames;
}

/// The map from names to graph CAs.
pub type GraphNames = BTreeMap<String, gantz_ca::CommitAddr>;

/// Commands emitted from the `GraphSelect` widget.
#[derive(Debug, Default)]
pub struct GraphSelectResponse {
    /// Indicates the new graph button was clicked.
    pub new_graph: bool,
    /// If a graph was selected this is its content address and name (if named).
    pub selected: Option<gantz_ca::Head>,
    /// The name was updated.
    pub name_updated: Option<Option<String>>,
    /// The name mapping was removed.
    pub name_removed: Option<String>,
}

/// Response returned from a row.
struct RowResponse {
    /// Response for the row.
    row: egui::Response,
    /// The response for the delete button.
    delete: Option<egui::Response>,
}

enum RowType<'a> {
    Named(&'a str),
    Unnamed(&'a gantz_ca::Timestamp),
}

impl<'a> GraphSelect<'a> {
    pub fn new(graph_reg: &'a dyn GraphRegistry, head: &'a gantz_ca::Head) -> Self {
        let id = egui::Id::new("gantz-graph-select");
        Self {
            graph_reg,
            head,
            id,
        }
    }

    pub fn with_id(mut self, id: egui::Id) -> Self {
        self.id = id;
        self
    }

    pub fn show(&mut self, ui: &mut egui::Ui) -> GraphSelectResponse {
        // Load any state specific to this widget (e.g. working text strings).
        let state_id = self.id.with("state");
        let mut state = ui
            .memory_mut(|mem| mem.data.get_temp::<GraphSelectState>(state_id))
            .unwrap_or_default();

        // If the given head has changed, reset the working name to the new
        // input.
        if Some(self.head.clone()) != state.last_head {
            state.last_head = Some(self.head.clone());
            match self.head {
                gantz_ca::Head::Branch(name) => state.working_graph_name = name.clone(),
                gantz_ca::Head::Commit(_) => state.working_graph_name.clear(),
            }
        }

        let mut response = GraphSelectResponse::default();

        // Allow for editing the name.
        // Show the currently selected head, allow using a name.
        let graph_names = self.graph_reg.names();
        let (head_name, head_ca): (Option<&str>, gantz_ca::CommitAddr) = match self.head {
            gantz_ca::Head::Branch(name) => (Some(name.as_str()), graph_names[name]),
            gantz_ca::Head::Commit(ca) => (None, *ca),
        };
        let (_name_res, new_name) =
            head_name_text_edit(head_name, head_ca, graph_names, &mut state, ui);
        if let Some(name_opt) = new_name.as_ref() {
            response.selected = match name_opt {
                None => Some(gantz_ca::Head::Commit(head_ca)),
                Some(new_name) => Some(gantz_ca::Head::Branch(new_name.clone())),
            };
        }
        response.name_updated = new_name.clone();

        ui.separator();

        // List all the graphs, named then unnamed.
        egui::ScrollArea::vertical()
            // Limit the scroll height to allow for the `+` button below.
            .max_height(
                ui.available_height() - ui.spacing().interact_size.y - ui.spacing().item_spacing.y,
            )
            // // Limit the scroll height to below the available height.
            // .max_height(available_h - ui.spacing().interact_size.y * 3.0)
            .show(ui, |ui| {
                // Show named graphs first.
                let mut visited = HashSet::new();
                for (name, &ca) in graph_names {
                    visited.insert(ca);
                    let res = graph_select_row(head_name, head_ca, RowType::Named(name), ca, ui);
                    if res.row.clicked() && ca != head_ca {
                        response.selected = Some(gantz_ca::Head::Branch(name.to_string()));
                    } else if let Some(delete) = res.delete {
                        if delete.clicked() {
                            response.name_removed = Some(name.to_string());
                        }
                    }
                }

                // Show remaining unnamed graphs.
                for (&ca, commit) in self
                    .graph_reg
                    .commits()
                    .into_iter()
                    .filter(|(ca, _)| !visited.contains(ca))
                {
                    // Use the timestamp as a row name.
                    let res = graph_select_row(
                        head_name,
                        head_ca,
                        RowType::Unnamed(&commit.timestamp),
                        ca,
                        ui,
                    );
                    if res.row.clicked() && ca != head_ca {
                        response.selected = Some(gantz_ca::Head::Commit(ca));
                    }
                }
            });

        ui.vertical_centered_justified(|ui| {
            response.new_graph |= ui.button("+").clicked();
        });

        // Store the modified state back in memory
        ui.memory_mut(|mem| mem.data.insert_temp(state_id, state));

        response
    }
}

// Allow for editing the name.
// Show the currently selected head, allow using a name.
fn head_name_text_edit(
    head_name: Option<&str>,
    head_ca: gantz_ca::CommitAddr,
    graph_names: &GraphNames,
    state: &mut GraphSelectState,
    ui: &mut egui::Ui,
) -> (egui::Response, Option<Option<String>>) {
    let ca_string = format!("{}", head_ca.display_short());

    // If this name is already assigned, but to a different CA, we'll colour the
    // name red and require `Ctrl + Enter` to overwrite the CA.
    let name_is_different = !(head_name.is_none() && state.working_graph_name.is_empty())
        && head_name != Some(&state.working_graph_name[..]);
    let name_is_taken = match graph_names.get(&state.working_graph_name) {
        Some(&ca) => ca != head_ca,
        None => false,
    };
    let hint_text = egui::RichText::new(&ca_string).monospace();
    let mut text_edit = egui::TextEdit::singleline(&mut state.working_graph_name)
        .desired_width(ui.available_width())
        .hint_text(hint_text);

    // If the name is taken, provide feedback via text color.
    if name_is_different && name_is_taken {
        text_edit = text_edit.text_color(egui::Color32::RED);
    }

    // Only update the name if its different.
    let name_res = ui.add(text_edit);
    let mut new_name = None;
    if name_res.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        // If the name is taken, only update if ctrl is down.
        let update = name_is_different && !name_is_taken || ui.input(|i| i.modifiers.ctrl);
        if update {
            new_name = if state.working_graph_name.is_empty() {
                Some(None)
            } else {
                Some(Some(state.working_graph_name.clone()))
            };
        }
    }
    (name_res, new_name)
}

fn graph_select_row(
    head_name_opt: Option<&str>,
    head_ca: gantz_ca::CommitAddr,
    row_type: RowType,
    row_ca: gantz_ca::CommitAddr,
    ui: &mut egui::Ui,
) -> RowResponse {
    let w = ui.max_rect().width();
    let h = ui.style().interaction.interact_radius;
    let size = egui::Vec2::new(w, h);
    let (rect, mut row) = ui.allocate_at_least(size, egui::Sense::click());

    let builder = egui::UiBuilder::new()
        .sense(egui::Sense::click())
        .max_rect(rect);
    let (res, delete) = ui
        .scope_builder(builder, |ui| {
            let mut res = ui.response();
            let hovered = res.hovered();

            // Create a child UI for the labels positioned over the allocated rect
            ui.horizontal(|ui| {
                let name = match row_type {
                    RowType::Named(name) => name.to_string(),
                    RowType::Unnamed(&timestamp) => fmt_commit_timestamp(timestamp),
                };
                let mut text = egui::RichText::new(name.clone());
                text = if row_ca == head_ca && Some(name.as_str()) == head_name_opt {
                    text.strong()
                } else if hovered {
                    text
                } else {
                    text.weak()
                };
                let label = egui::Label::new(text).selectable(false);
                res |= ui.add(label);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Show the address.
                    let row_ca_string = format!("{}", row_ca.display_short());
                    let mut text = egui::RichText::new(row_ca_string).monospace();
                    text = if row_ca == head_ca {
                        text.strong()
                    } else if hovered {
                        text
                    } else {
                        text.weak()
                    };
                    let label = egui::Label::new(text).selectable(false);
                    res |= ui.add(label);

                    // Show an x for removing the name mapping.
                    let delete = match row_type {
                        RowType::Named(_) => {
                            let res = ui.add(egui::Button::new("×").frame_when_inactive(false));
                            Some(res)
                        }
                        RowType::Unnamed(_) => None,
                    };
                    (res, delete)
                })
                .inner
            })
            .inner
        })
        .inner;

    row |= res;

    RowResponse { row, delete }
}

// Format the commit as a timestamp for listing unnamed commits.
fn fmt_commit_timestamp(timestamp: gantz_ca::Timestamp) -> String {
    std::time::UNIX_EPOCH
        .checked_add(timestamp)
        .map(|time| humantime::format_rfc3339_seconds(time).to_string())
        .unwrap_or_else(|| "<invalid-timestamp>".to_string())
}
