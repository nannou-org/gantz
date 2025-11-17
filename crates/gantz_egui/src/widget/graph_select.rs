//! A simple widget for selecting between, naming and creating new graphs.

use std::collections::HashSet;

/// A widget for selecting between, naming, and creating new graphs.
pub struct GraphSelect<'a> {
    id: egui::Id,
    registry: &'a dyn GraphRegistry,
    heads: &'a [gantz_ca::Head],
}

#[derive(Clone, Default)]
struct GraphSelectState {
    name_filter: String,
}

/// Methods required on the provided graph registry.
pub trait GraphRegistry {
    /// All selectable commit addresses.
    fn commits(&self) -> Vec<(&gantz_ca::CommitAddr, &gantz_ca::Commit)>;
    /// An iterator yielding all name -> CA pairs.
    fn names(&self) -> &gantz_ca::registry::Names;
}

/// Commands emitted from the `GraphSelect` widget.
#[derive(Debug, Default)]
pub struct GraphSelectResponse {
    /// Indicates the new graph button was clicked.
    pub new_graph: bool,
    /// If a graph was selected this is its head.
    pub selected: Option<gantz_ca::Head>,
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
    pub fn new(registry: &'a dyn GraphRegistry, heads: &'a [gantz_ca::Head]) -> Self {
        let id = egui::Id::new("gantz-graph-select");
        Self {
            registry,
            heads,
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

        let mut response = GraphSelectResponse::default();

        // A text edit for filtering names.
        egui::TextEdit::singleline(&mut state.name_filter)
            .desired_width(ui.available_width())
            .hint_text("🔎 Name Filter")
            .show(ui);

        let names = self.registry.names();

        // List all the graphs, named then unnamed.
        egui::ScrollArea::vertical()
            // Limit the scroll height to allow for the `+` button below.
            .max_height(
                ui.available_height() - ui.spacing().interact_size.y - ui.spacing().item_spacing.y,
            )
            .show(ui, |ui| {
                // Show named graphs first.
                let mut visited = HashSet::new();
                for (name, ca) in names {
                    if !state.name_filter.is_empty()
                        && !state.name_filter.split(" ").all(|s| name.contains(s))
                    {
                        continue;
                    }
                    visited.insert(ca);
                    let res = graph_select_row(self.heads, RowType::Named(name), ca, &names, ui);
                    if res.row.clicked() {
                        response.selected = Some(gantz_ca::Head::Branch(name.to_string()));
                    } else if let Some(delete) = res.delete {
                        if delete.clicked() {
                            response.name_removed = Some(name.to_string());
                        }
                    }
                }

                // Show remaining unnamed graphs.
                for (ca, commit) in self
                    .registry
                    .commits()
                    .into_iter()
                    .filter(|(ca, _)| !visited.contains(ca))
                {
                    if !state.name_filter.is_empty() {
                        let ca_str = format!("{ca}");
                        if !state.name_filter.split(" ").all(|s| ca_str.contains(s)) {
                            continue;
                        }
                    }

                    // Use the timestamp as a row name.
                    let row_type = RowType::Unnamed(&commit.timestamp);
                    let res = graph_select_row(self.heads, row_type, ca, &names, ui);
                    if res.row.clicked() {
                        response.selected = Some(gantz_ca::Head::Commit(*ca));
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

fn heads_contain(
    heads: &[gantz_ca::Head],
    row_ca: &gantz_ca::CommitAddr,
    names: &gantz_ca::registry::Names,
) -> bool {
    heads.iter().any(|head| match head {
        gantz_ca::Head::Commit(ca) => row_ca == ca,
        gantz_ca::Head::Branch(name) => names.get(name) == Some(row_ca),
    })
}

fn graph_select_row(
    heads: &[gantz_ca::Head],
    row_type: RowType,
    row_ca: &gantz_ca::CommitAddr,
    names: &gantz_ca::registry::Names,
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
                let heads_contain = heads_contain(heads, row_ca, names);
                text = if heads_contain {
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
                    text = if heads_contain {
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
