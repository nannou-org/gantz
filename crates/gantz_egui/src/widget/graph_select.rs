//! A simple widget for selecting between, naming and creating new graphs.

use super::head_row::{HeadRowType, head_row};
use std::collections::HashSet;

/// A widget for selecting between, naming, and creating new graphs.
pub struct GraphSelect<'a> {
    id: egui::Id,
    registry: &'a dyn GraphRegistry,
    heads: &'a [gantz_ca::Head],
    focused_head: Option<usize>,
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
    /// Single click: replace the focused head with this one.
    pub replaced: Option<gantz_ca::Head>,
    /// Ctrl+click on a head that is not open: open this head as a new tab.
    pub opened: Option<gantz_ca::Head>,
    /// Ctrl+click on a head that is already open: close this head.
    pub closed: Option<gantz_ca::Head>,
    /// The name mapping was removed.
    pub name_removed: Option<String>,
}

impl GraphSelectResponse {
    /// Combine two responses, preferring `Some` values from `other`.
    pub fn union(self, other: Self) -> Self {
        Self {
            new_graph: self.new_graph || other.new_graph,
            replaced: other.replaced.or(self.replaced),
            opened: other.opened.or(self.opened),
            closed: other.closed.or(self.closed),
            name_removed: other.name_removed.or(self.name_removed),
        }
    }
}

impl std::ops::BitOr for GraphSelectResponse {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

impl std::ops::BitOrAssign for GraphSelectResponse {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = std::mem::take(self).union(rhs);
    }
}

impl<'a> GraphSelect<'a> {
    pub fn new(registry: &'a dyn GraphRegistry, heads: &'a [gantz_ca::Head]) -> Self {
        let id = egui::Id::new("gantz-graph-select");
        Self {
            registry,
            heads,
            id,
            focused_head: None,
        }
    }

    pub fn with_id(mut self, id: egui::Id) -> Self {
        self.id = id;
        self
    }

    /// Set the index of the focused head to show a focus indicator.
    pub fn focused_head(mut self, focused_head: usize) -> Self {
        self.focused_head = Some(focused_head);
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
                        && !state
                            .name_filter
                            .split_whitespace()
                            .all(|s| name.contains(s))
                    {
                        continue;
                    }
                    visited.insert(ca);
                    let head = gantz_ca::Head::Branch(name.to_string());
                    let res = head_row(
                        self.heads,
                        &head,
                        HeadRowType::Named(name),
                        ca,
                        self.focused_head,
                        ui,
                    );
                    if res.row.clicked() {
                        let ctrl = ui.input(|i| i.modifiers.ctrl);
                        if ctrl {
                            if self.heads.contains(&head) {
                                response.closed = Some(head);
                            } else {
                                response.opened = Some(head);
                            }
                        } else {
                            response.replaced = Some(head);
                        }
                    } else if let Some(delete) = res.delete {
                        if delete.clicked() {
                            response.name_removed = Some(name.to_string());
                        }
                    }
                }

                // Collect commit addresses for open heads (excluding named ones already shown).
                let open_head_cas: HashSet<_> = self
                    .heads
                    .iter()
                    .filter_map(|head| match head {
                        gantz_ca::Head::Branch(_) => None, // Already shown in named section
                        gantz_ca::Head::Commit(ca) => Some(*ca),
                    })
                    .collect();

                // Show only unnamed commits that are currently open as heads.
                for (ca, commit) in self
                    .registry
                    .commits()
                    .into_iter()
                    .filter(|(ca, _)| !visited.contains(ca) && open_head_cas.contains(ca))
                {
                    if !state.name_filter.is_empty() {
                        let ca_str = format!("{ca}");
                        if !state.name_filter.split(" ").all(|s| ca_str.contains(s)) {
                            continue;
                        }
                    }

                    // Use the timestamp as a row name.
                    let head = gantz_ca::Head::Commit(*ca);
                    let row_type = HeadRowType::Unnamed(&commit.timestamp);
                    let res = head_row(self.heads, &head, row_type, ca, self.focused_head, ui);
                    if res.row.clicked() {
                        let ctrl = ui.input(|i| i.modifiers.ctrl);
                        if ctrl {
                            if self.heads.contains(&head) {
                                response.closed = Some(head);
                            } else {
                                response.opened = Some(head);
                            }
                        } else {
                            response.replaced = Some(head);
                        }
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
