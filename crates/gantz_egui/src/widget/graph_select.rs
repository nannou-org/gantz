//! A simple widget for selecting between, naming and creating new graphs.

use crate::{ContentAddr, fmt_content_addr, fmt_content_addr_short};
use std::collections::{BTreeMap, HashSet};

/// A widget for selecting between, naming, and creating new graphs.
pub struct GraphSelect<'a> {
    id: egui::Id,
    graph_reg: &'a dyn GraphRegistry,
    head: Head<'a>,
}

#[derive(Clone, Default)]
struct GraphSelectState {
    /// The last head name provided via argument. We track this to know if we
    /// should reset the working graph name.
    last_head_name: Option<String>,
    working_graph_name: String,
}

/// The currently active graph.
#[derive(Clone, Copy)]
pub struct Head<'a> {
    /// The currently selected graph's CA.
    pub ca: ContentAddr,
    /// The currently active name.
    pub name: Option<&'a str>,
}

/// Methods required on the provided graph registry.
pub trait GraphRegistry {
    /// All selectable graph addresses.
    fn addrs(&self) -> Vec<ContentAddr>;
    /// An iterator yielding all name -> CA pairs.
    fn names(&self) -> &GraphNames;
}

/// The map from names to graph CAs.
pub type GraphNames = BTreeMap<String, ContentAddr>;

/// Commands emitted from the `GraphSelect` widget.
#[derive(Debug, Default)]
pub struct GraphSelectResponse {
    /// Indicates the new graph button was clicked.
    pub new_graph: bool,
    /// If a graph was selected this is its content address and name (if named).
    pub selected: Option<(Option<String>, ContentAddr)>,
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

impl<'a> GraphSelect<'a> {
    pub fn new(graph_reg: &'a dyn GraphRegistry, head: Head<'a>) -> Self {
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

        // If the given active name has changed, reset the working name to the
        // new input.
        if state.last_head_name.as_deref() != self.head.name {
            state.last_head_name = self.head.name.map(str::to_string);
            state.working_graph_name = self.head.name.map(str::to_string).unwrap_or_default();
        }

        let mut response = GraphSelectResponse::default();

        // Allow for editing the name.
        // Show the currently selected head, allow using a name.
        let graph_names = self.graph_reg.names();
        let (_name_res, new_name) = head_name_text_edit(&self.head, graph_names, &mut state, ui);
        if let Some(name_opt) = new_name.as_ref() {
            response.selected = Some((name_opt.clone(), self.head.ca));
        }
        response.name_updated = new_name.clone();

        ui.separator();

        // List all the graphs, named then unnamed.
        egui::ScrollArea::vertical().show(ui, |ui| {
            // Show named graphs first.
            let mut visited = HashSet::new();
            for (name, &ca) in graph_names {
                visited.insert(ca);
                let res = graph_select_row(&self.head, Some(name), ca, ui);
                if res.row.clicked() && ca != self.head.ca {
                    response.selected = Some((Some(name.to_string()), ca));
                } else if let Some(delete) = res.delete {
                    if delete.clicked() {
                        response.name_removed = Some(name.to_string());
                    }
                }
            }

            // Show remaining unnamed graphs.
            for ca in self
                .graph_reg
                .addrs()
                .into_iter()
                .filter(|ca| !visited.contains(ca))
            {
                let res = graph_select_row(&self.head, None, ca, ui);
                if res.row.clicked() && ca != self.head.ca {
                    response.selected = Some((None, ca));
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
    head: &Head,
    graph_names: &GraphNames,
    state: &mut GraphSelectState,
    ui: &mut egui::Ui,
) -> (egui::Response, Option<Option<String>>) {
    let ca_string = fmt_content_addr(head.ca);

    // If this name is already assigned, but to a different CA, we'll colour the
    // name red and require `Ctrl + Enter` to overwrite the CA.
    let name_is_different = !(head.name.is_none() && state.working_graph_name.is_empty())
        && head.name != Some(&state.working_graph_name[..]);
    let name_is_taken = match graph_names.get(&state.working_graph_name) {
        Some(&ca) => ca != head.ca,
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
    head: &Head,
    name_opt: Option<&str>,
    ca: ContentAddr,
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
                let name = name_opt.unwrap_or("----------");
                let mut text = egui::RichText::new(name.to_string());
                text = if ca == head.ca && Some(name) == head.name {
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
                    let mut text = egui::RichText::new(fmt_content_addr_short(ca)).monospace();
                    text = if ca == head.ca {
                        text.strong()
                    } else if hovered {
                        text
                    } else {
                        text.weak()
                    };
                    let label = egui::Label::new(text).selectable(false);
                    res |= ui.add(label);

                    // Show an x for removing the name mapping.
                    let delete = name_opt
                        .is_some()
                        .then(|| ui.add(egui::Button::new("×").frame_when_inactive(false)));

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
