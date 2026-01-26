//! A widget for viewing commit history with All/Focused mode.

use super::graph_select::{GraphRegistry, GraphSelectResponse};
use super::head_row::{HeadRowType, head_row};
use std::collections::HashMap;

/// A widget for viewing commit history.
pub struct HistoryView<'a> {
    id: egui::Id,
    registry: &'a dyn GraphRegistry,
    heads: &'a [gantz_ca::Head],
    focused_head: Option<usize>,
}

/// Persistent state for the HistoryView widget.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct HistoryViewState {
    pub mode: HistoryMode,
}

/// The display mode for the history view.
#[derive(Clone, Copy, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum HistoryMode {
    /// Show all commits in the repository.
    #[default]
    All,
    /// Show only commits in the parent chain of the focused head.
    Focused,
}

impl<'a> HistoryView<'a> {
    pub fn new(registry: &'a dyn GraphRegistry, heads: &'a [gantz_ca::Head]) -> Self {
        let id = egui::Id::new("gantz-history-view");
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

    /// Set the index of the focused head.
    pub fn focused_head(mut self, focused_head: usize) -> Self {
        self.focused_head = Some(focused_head);
        self
    }

    pub fn show(&mut self, ui: &mut egui::Ui) -> GraphSelectResponse {
        // Load state.
        let state_id = self.id.with("state");
        let mut state = ui
            .memory_mut(|mem| mem.data.get_temp::<HistoryViewState>(state_id))
            .unwrap_or_default();

        let mut response = GraphSelectResponse::default();

        // Mode toggle at top.
        ui.horizontal(|ui| {
            ui.radio_value(&mut state.mode, HistoryMode::All, "All")
                .on_hover_text("Show all commits in the registry");

            ui.radio_value(&mut state.mode, HistoryMode::Focused, "Focused")
                .on_hover_ui(|ui| {
                    let text = self
                        .focused_head
                        .and_then(|idx| self.heads.get(idx))
                        .map(|head| match head {
                            gantz_ca::Head::Branch(name) => {
                                format!("Show commits in the parent chain of '{name}'")
                            }
                            gantz_ca::Head::Commit(ca) => {
                                format!(
                                    "Show commits in the parent chain of {}",
                                    ca.display_short()
                                )
                            }
                        })
                        .unwrap_or_else(|| {
                            "Show commits in the parent chain of the focused graph".into()
                        });
                    ui.label(text);
                });
        });

        // Get commits based on mode.
        let commits = self.registry.commits();
        let commit_map: HashMap<_, _> = commits.iter().map(|(ca, c)| (*ca, *c)).collect();

        // Build set of commit addresses to show based on mode.
        let filtered_cas: Option<std::collections::HashSet<gantz_ca::CommitAddr>> = match state.mode
        {
            HistoryMode::All => None, // Show all
            HistoryMode::Focused => {
                // Get the focused head's commit address and walk parent chain.
                let focused_ca = self.focused_head.and_then(|idx| {
                    self.heads.get(idx).and_then(|head| match head {
                        gantz_ca::Head::Branch(name) => self.registry.names().get(name).copied(),
                        gantz_ca::Head::Commit(ca) => Some(*ca),
                    })
                });

                if let Some(start_ca) = focused_ca {
                    // Walk the parent chain and collect addresses.
                    let mut chain = std::collections::HashSet::new();
                    let mut current = Some(start_ca);
                    while let Some(ca) = current {
                        chain.insert(ca);
                        current = commit_map.get(&ca).and_then(|c| c.parent);
                    }
                    Some(chain)
                } else {
                    Some(std::collections::HashSet::new())
                }
            }
        };

        // Filter commits if in focused mode.
        let filtered_commits: Vec<_> = match &filtered_cas {
            None => commits,
            Some(cas) => commits
                .into_iter()
                .filter(|(ca, _)| cas.contains(ca))
                .collect(),
        };

        // Show commits in scroll area.
        egui::ScrollArea::vertical()
            .auto_shrink(egui::Vec2b::FALSE)
            .show(ui, |ui| {
                for (ca, commit) in filtered_commits {
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

        // Store state.
        ui.memory_mut(|mem| mem.data.insert_temp(state_id, state));

        response
    }
}
