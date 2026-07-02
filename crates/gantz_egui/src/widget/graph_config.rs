//! A widget for configuring per-head graph layout settings and renaming.

use super::gantz::OpenHeadState;
use super::head_name_edit::{head_name, head_name_edit};

/// Per-head graph configuration widget.
///
/// Provides a name-editing text field, one-shot `auto-layout`/`center view`
/// buttons, and the per-head layout flow direction. The non-flow layout
/// parameters live globally in `Settings > Global`.
pub struct GraphConfig<'a> {
    head: &'a gantz_ca::Head,
    head_state: &'a mut OpenHeadState,
    names: &'a gantz_ca::registry::Names,
    is_base: bool,
    immutable: bool,
    demo_names: &'a [&'a str],
    current_demo: Option<&'a str>,
    current_description: Option<&'a str>,
    env: Option<(
        &'a dyn crate::Registry,
        &'a mut gantz_ca::merge::Resolutions,
    )>,
}

/// Response from the [`GraphConfig`] widget.
pub struct GraphConfigResponse {
    /// A new branch name was committed via the name editor.
    pub new_branch: Option<(gantz_ca::Head, String)>,
    /// The "Export" button was clicked.
    pub export: bool,
    /// Demo graph association changed: `Some(Some(name))` = set, `Some(None)` = clear.
    pub demo_changed: Option<Option<String>>,
    /// The "Reset" button was clicked for a base graph.
    pub reset_base_graph: bool,
    /// The graph's description was edited (committed on focus loss). An empty
    /// string clears the description.
    pub description_changed: Option<String>,
    /// A merge candidate was chosen from the merge row.
    pub merge: Option<crate::MergeHead>,
}

impl<'a> GraphConfig<'a> {
    pub fn new(
        head: &'a gantz_ca::Head,
        head_state: &'a mut OpenHeadState,
        names: &'a gantz_ca::registry::Names,
    ) -> Self {
        Self {
            head,
            head_state,
            names,
            is_base: false,
            immutable: false,
            demo_names: &[],
            current_demo: None,
            current_description: None,
            env: None,
        }
    }

    /// Whether this graph is a base node - a pre-composed graph that ships
    /// with the binary and is reset to its original form on every launch.
    /// Users who want to customize a base node should duplicate it under a
    /// new name.
    pub fn is_base(mut self, is_base: bool) -> Self {
        self.is_base = is_base;
        self
    }

    /// Whether this graph is immutable - layout controls will be disabled.
    pub fn immutable(mut self, immutable: bool) -> Self {
        self.immutable = immutable;
        self
    }

    /// Available demo graph names for the dropdown.
    pub fn demo_names(mut self, demo_names: &'a [&'a str]) -> Self {
        self.demo_names = demo_names;
        self
    }

    /// The current demo graph association for this graph, if any.
    pub fn current_demo(mut self, current_demo: Option<&'a str>) -> Self {
        self.current_demo = current_demo;
        self
    }

    /// The graph's current description, used to seed the description editor.
    pub fn current_description(mut self, current_description: Option<&'a str>) -> Self {
        self.current_description = current_description;
        self
    }

    /// Registry access for the merge row's candidates and previews, plus the
    /// conflict-resolution strategy its "⛭" menu edits. Without these, the
    /// merge row is hidden.
    pub fn merge_env(
        mut self,
        env: &'a dyn crate::Registry,
        resolutions: &'a mut gantz_ca::merge::Resolutions,
    ) -> Self {
        self.env = Some((env, resolutions));
        self
    }

    pub fn show(self, ui: &mut egui::Ui) -> GraphConfigResponse {
        let is_named = matches!(self.head, gantz_ca::Head::Branch(_));
        let is_demo =
            matches!(&self.head, gantz_ca::Head::Branch(name) if name.starts_with("demo-"));

        // Per-head temp state for the name editor.
        let edit_id = egui::Id::new("graph_config_name_edit").with(self.head);
        let mut name = ui
            .memory_mut(|m| m.data.get_temp::<String>(edit_id))
            .unwrap_or_else(|| head_name(self.head));

        // Per-head temp state for the description editor (named graphs only).
        // The edit is committed on focus loss to avoid a commit per keypress.
        let desc_id = egui::Id::new("graph_config_desc_edit").with(self.head);
        let mut desc = is_named.then(|| {
            let current = self.current_description.unwrap_or("");
            ui.memory_mut(|m| m.data.get_temp::<String>(desc_id))
                .unwrap_or_else(|| current.to_string())
        });

        // Outputs collected from within the grid.
        let mut new_branch = None;
        let mut description_changed = None;
        let mut demo_changed = None;
        let mut reset_base_graph = false;
        let mut export = false;
        let mut merge = None;

        // Reserve room for the label column so the value column's text fields
        // don't expand to fill the entire pane.
        let control_w = (ui.available_width() - 64.0).max(64.0);

        egui::Grid::new(egui::Id::new("graph_config_grid").with(self.head))
            .num_columns(2)
            .spacing([8.0, 6.0])
            .striped(true)
            .show(ui, |ui| {
                // name
                ui.label("name");
                new_branch = ui
                    .scope(|ui| {
                        ui.set_max_width(control_w);
                        head_name_edit(self.head, &mut name, self.names, ui)
                    })
                    .inner
                    .new_branch;
                ui.end_row();

                // desc.
                if let Some(desc) = desc.as_mut() {
                    ui.label("desc.");
                    let current = self.current_description.unwrap_or("");
                    // Multiline + word-wrap; the grid row auto-fits its height
                    // to the (wrapped) text, growing from a single row.
                    let resp = ui.add_enabled(
                        !self.immutable,
                        egui::TextEdit::multiline(desc)
                            .hint_text("Description")
                            .desired_rows(1)
                            .desired_width(control_w),
                    );
                    if resp.lost_focus() && desc.as_str() != current {
                        description_changed = Some(desc.clone());
                    }
                    ui.end_row();
                }

                // demo (named, non-demo graphs only)
                if is_named && !is_demo && !self.demo_names.is_empty() {
                    ui.label("demo");
                    ui.add_enabled_ui(!self.immutable, |ui| {
                        let selected_text = self.current_demo.unwrap_or("none");
                        egui::ComboBox::from_id_salt("demo_graph_select")
                            .selected_text(selected_text)
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_label(self.current_demo.is_none(), "none")
                                    .clicked()
                                {
                                    demo_changed = Some(None);
                                }
                                for &demo_name in self.demo_names {
                                    if ui
                                        .selectable_label(
                                            self.current_demo == Some(demo_name),
                                            demo_name,
                                        )
                                        .clicked()
                                    {
                                        demo_changed = Some(Some(demo_name.to_string()));
                                    }
                                }
                            });
                    });
                    ui.end_row();
                }

                // reset (base demo graphs only)
                if self.is_base && is_demo {
                    ui.label("reset");
                    if ui
                        .button("Reset")
                        .on_hover_text("reset demo to initial state")
                        .clicked()
                    {
                        reset_base_graph = true;
                    }
                    ui.end_row();
                }

                // base note
                if self.is_base {
                    ui.label("");
                    ui.label(
                        egui::RichText::new("\"base\" node, included with gantz")
                            .italics()
                            .weak(),
                    );
                    ui.end_row();
                }

                // layout - center-view and auto-layout side by side. Both are
                // one-shot: they apply once when clicked (consumed by the graph
                // scene next pass), so hand-arranged nodes are never disturbed.
                ui.label("layout");
                ui.horizontal(|ui| {
                    if ui
                        .button("center view")
                        .on_hover_text("center the view over the graph")
                        .clicked()
                    {
                        self.head_state.scene.pending_center_view = true;
                    }
                    ui.add_enabled_ui(!self.immutable, |ui| {
                        if ui
                            .button("auto-layout")
                            .on_hover_text(
                                "lay out the selection, or the whole graph when nothing is selected",
                            )
                            .clicked()
                        {
                            self.head_state.scene.pending_auto_layout = true;
                        }
                    });
                });
                ui.end_row();

                // flow
                ui.label("flow");
                ui.add_enabled_ui(!self.immutable, |ui| {
                    ui.horizontal(|ui| {
                        ui.radio_value(
                            &mut self.head_state.layout_flow,
                            egui::Direction::LeftToRight,
                            "Right",
                        );
                        ui.radio_value(
                            &mut self.head_state.layout_flow,
                            egui::Direction::TopDown,
                            "Down",
                        );
                    });
                });
                ui.end_row();

                // merge (named, mutable, non-base graphs only)
                if is_named && !self.immutable && !self.is_base {
                    if let Some((env, resolutions)) = self.env {
                        ui.label("merge");
                        ui.horizontal(|ui| {
                            merge = merge_select(env, self.head, *resolutions, ui);
                            ui.menu_button("\u{26ED}", |ui| {
                                resolutions_menu(resolutions, ui);
                            })
                            .response
                            .on_hover_text("conflict resolution strategy");
                        });
                        ui.end_row();
                    }
                }

                // export
                ui.label("export");
                export = ui
                    .button("export")
                    .on_hover_text("export this graph and its dependencies to a .gantz file")
                    .clicked();
                ui.end_row();
            });

        // Persist the per-head editor buffers.
        ui.memory_mut(|m| m.data.insert_temp(edit_id, name));
        if let Some(desc) = desc {
            ui.memory_mut(|m| m.data.insert_temp(desc_id, desc));
        }

        GraphConfigResponse {
            new_branch,
            export,
            demo_changed,
            reset_base_graph,
            description_changed,
            merge,
        }
    }
}

/// The merge row's branch selector: a dropdown of merge candidates, each with
/// a dry-run hover summary. Picking a candidate *is* the action (one-shot,
/// like the layout buttons): a clean candidate merges on click; a conflicted
/// one is disabled, its tooltip listing the conflicts, with a separate opt-in
/// button applying the selected [`Resolutions`]. Hard-blocked candidates
/// (e.g. a reference cycle) can only be inspected.
///
/// Candidates and previews are only computed while the popup is open, and each
/// preview is cached keyed by the two branch tips (content addresses) plus the
/// resolution strategy, so a cached preview can never go stale.
///
/// [`Resolutions`]: gantz_ca::merge::Resolutions
fn merge_select(
    env: &dyn crate::Registry,
    head: &gantz_ca::Head,
    resolutions: gantz_ca::merge::Resolutions,
    ui: &mut egui::Ui,
) -> Option<crate::MergeHead> {
    let mut merge = None;
    egui::ComboBox::from_id_salt("merge_select")
        .selected_text("select branch\u{2026}")
        .show_ui(ui, |ui| {
            let candidates = env.merge_candidates(head);
            if candidates.is_empty() {
                ui.weak("no mergeable graphs");
                return;
            }
            let ours_tip = match head {
                gantz_ca::Head::Branch(name) => env.names().get(name).copied(),
                gantz_ca::Head::Commit(ca) => Some(*ca),
            };
            for candidate in candidates {
                // Fetch (or compute and cache) the candidate's dry-run preview.
                let preview_id =
                    egui::Id::new("merge_preview").with((ours_tip, candidate.theirs, resolutions));
                let preview = ui
                    .memory_mut(|m| m.data.get_temp::<crate::merge::MergePreview>(preview_id))
                    .or_else(|| {
                        let preview = env.merge_preview(head, &candidate.name, resolutions);
                        if let Some(preview) = &preview {
                            ui.memory_mut(|m| m.data.insert_temp(preview_id, preview.clone()));
                        }
                        preview
                    });

                let mut summary = preview
                    .as_ref()
                    .map(|p| crate::merge::summary_text(&p.summary))
                    .unwrap_or_default();
                if candidate.fast_forward {
                    summary =
                        format!("fast-forward: moves this graph to the branch tip\n{summary}");
                }
                let clean = preview.as_ref().is_none_or(|p| p.is_clean());
                if clean {
                    let mut text = candidate.name.clone();
                    if candidate.fast_forward {
                        text.push_str(" (fast-forward)");
                    }
                    if ui
                        .selectable_label(false, text)
                        .on_hover_text(summary)
                        .clicked()
                    {
                        merge = Some(crate::MergeHead {
                            source: candidate.name.clone(),
                            resolutions,
                            auto_resolve: false,
                        });
                    }
                    continue;
                }

                // Conflicted or blocked: not directly selectable. Conflicts
                // (but not blockers) offer an explicit opt-in that applies the
                // selected resolutions.
                let preview = preview.expect("`!clean` requires a preview");
                let warn = crate::node::named_ref::outdated_color();
                ui.horizontal(|ui| {
                    let text = egui::RichText::new(format!("{} !", candidate.name)).color(warn);
                    ui.add_enabled(false, egui::Button::selectable(false, text))
                        .on_disabled_hover_ui(|ui| {
                            ui.set_max_width(320.0);
                            ui.label(summary);
                            for conflict in &preview.conflicts {
                                ui.colored_label(warn, format!("! {conflict}"));
                            }
                            for blocker in &preview.blockers {
                                ui.colored_label(
                                    crate::node::named_ref::missing_color(),
                                    format!("\u{2715} {blocker}"),
                                );
                            }
                        });
                    if preview.blockers.is_empty()
                        && ui
                            .small_button("merge anyway")
                            .on_hover_text(
                                "merge despite the conflicts, applying the selected \
                                 resolutions (see \u{26ED})",
                            )
                            .clicked()
                    {
                        merge = Some(crate::MergeHead {
                            source: candidate.name.clone(),
                            resolutions,
                            auto_resolve: true,
                        });
                    }
                });
            }
        });
    merge
}

/// The "⛭" menu beside the merge dropdown: how conflicts resolve when merging
/// despite them. Edits the persisted, GUI-global strategy in place.
fn resolutions_menu(resolutions: &mut gantz_ca::merge::Resolutions, ui: &mut egui::Ui) {
    use gantz_ca::merge::{EditOrDelete, Side};
    ui.label("when both sides modified a node");
    ui.radio_value(
        &mut resolutions.both_modified,
        Side::Ours,
        "keep this graph's version",
    );
    ui.radio_value(
        &mut resolutions.both_modified,
        Side::Theirs,
        "keep the branch's version",
    );
    ui.separator();
    ui.label("when a delete meets an edit");
    ui.radio_value(
        &mut resolutions.delete_modify,
        EditOrDelete::KeepEdit,
        "keep the edited node",
    );
    ui.radio_value(
        &mut resolutions.delete_modify,
        EditOrDelete::KeepDelete,
        "keep the delete",
    );
}
