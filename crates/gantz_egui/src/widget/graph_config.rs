//! A widget for configuring per-head graph layout settings and renaming.

use super::gantz::OpenHeadState;
use super::head_name_edit::{head_name, head_name_edit};
use crate::Registry;
use gantz_core::node;

/// Per-head graph configuration widget.
///
/// Provides a name-editing text field and layout settings
/// (`auto_layout`, `layout_flow`, `center_view`).
pub struct GraphConfig<'a> {
    head: &'a gantz_ca::Head,
    head_state: &'a mut OpenHeadState,
    names: &'a gantz_ca::registry::Names,
    registry: &'a dyn Registry,
    is_base: bool,
    immutable: bool,
    demo_names: &'a [&'a str],
    current_demo: Option<&'a str>,
    persist_state: bool,
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
    /// The "Persist State" toggle changed (Some(new_value) if changed).
    pub persist_state_changed: Option<bool>,
}

impl<'a> GraphConfig<'a> {
    pub fn new(
        head: &'a gantz_ca::Head,
        head_state: &'a mut OpenHeadState,
        names: &'a gantz_ca::registry::Names,
        registry: &'a dyn Registry,
    ) -> Self {
        Self {
            head,
            head_state,
            names,
            registry,
            is_base: false,
            immutable: false,
            demo_names: &[],
            current_demo: None,
            persist_state: false,
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

    /// Whether state persistence is currently enabled for this graph.
    pub fn persist_state(mut self, persist_state: bool) -> Self {
        self.persist_state = persist_state;
        self
    }

    pub fn show(self, ui: &mut egui::Ui) -> GraphConfigResponse {
        // Export button (right) + name field (filling remaining space).
        let (export, new_branch) = ui
            .horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let export = ui
                        .button("\u{2B07}")
                        .on_hover_text("export this graph")
                        .clicked();
                    let edit_id = egui::Id::new("graph_config_name_edit").with(self.head);
                    let mut name = ui
                        .memory_mut(|m| m.data.get_temp::<String>(edit_id))
                        .unwrap_or_else(|| head_name(self.head));
                    let name_res = head_name_edit(self.head, &mut name, self.names, ui);
                    ui.memory_mut(|m| m.data.insert_temp(edit_id, name));
                    (export, name_res.new_branch)
                })
                .inner
            })
            .inner;

        // Persist State toggle - visible only for named, stateful, non-base graphs.
        let is_stateful = match self.head {
            gantz_ca::Head::Branch(name) => self
                .names
                .get(name)
                .map(|commit_ca| {
                    let ca = gantz_ca::ContentAddr::from(*commit_ca);
                    let get_node = |ca: &gantz_ca::ContentAddr| self.registry.node(ca);
                    let meta_ctx = node::MetaCtx::new(&get_node);
                    self.registry
                        .node(&ca)
                        .map(|n| n.stateful(meta_ctx))
                        .unwrap_or(false)
                })
                .unwrap_or(false),
            _ => false,
        };
        let is_named = matches!(self.head, gantz_ca::Head::Branch(_));
        let show_persist = is_stateful && is_named && !self.is_base;
        let mut persist_state_changed = None;
        if show_persist {
            let mut persist = self.persist_state;
            if ui.checkbox(&mut persist, "Persist State").changed() {
                persist_state_changed = Some(persist);
            }
        }

        // Demo graph selector (only for named, non-demo graphs).
        let is_named = matches!(self.head, gantz_ca::Head::Branch(_));
        let is_demo =
            matches!(&self.head, gantz_ca::Head::Branch(name) if name.starts_with("demo-"));
        let mut demo_changed = None;
        if is_named && !is_demo && !self.demo_names.is_empty() {
            ui.add_enabled_ui(!self.immutable, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Demo:");
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
            });
        }

        // Reset button for demo base graphs.
        let mut reset_base_graph = false;
        if self.is_base && is_demo {
            if ui
                .button("Reset")
                .on_hover_text("reset demo to initial state")
                .clicked()
            {
                reset_base_graph = true;
            }
        }

        if self.is_base {
            ui.label(
                egui::RichText::new("\"base\" node, included with gantz")
                    .italics()
                    .weak(),
            );
        }

        // Layout config.
        ui.checkbox(&mut self.head_state.center_view, "Center View");
        ui.add_enabled_ui(!self.immutable, |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.head_state.auto_layout, "Automatic Layout");
            });
            ui.horizontal(|ui| {
                ui.label("Flow:");
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

        GraphConfigResponse {
            new_branch,
            export,
            demo_changed,
            reset_base_graph,
            persist_state_changed,
        }
    }
}
