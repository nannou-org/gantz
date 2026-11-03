//! A simple widget for selecting between, naming and creating new graphs.

use super::head_row::{HeadRowType, head_row};
use gantz_ca::Name;
use std::collections::HashSet;

/// The glyph for the filter-options button. Swap it if it does not render.
const FILTER_GLYPH: &str = "⛭";

/// The gap between the filter, the list and the buttons below it.
const SECTION_GAP: f32 = 6.0;

/// The gap between the rows of the list.
const ROW_GAP: f32 = 6.0;

/// A widget for selecting between, naming, and creating new graphs.
pub struct GraphSelect<'a> {
    id: egui::Id,
    registry: &'a crate::Env<'a>,
    heads: &'a [gantz_ca::Head],
    focused_head: Option<usize>,
    base_names: &'a crate::reg::Names,
    /// Collaborative-session display state, when a collab layer is wired. It
    /// enables the join button and the per-row session dots.
    collab: Option<&'a crate::collab::CollabUiState>,
    /// A host-provided clipboard reader for the join popup's right-click
    /// paste. egui alone cannot read the clipboard.
    clipboard: Option<&'a dyn Fn() -> Option<String>>,
}

#[derive(Clone)]
struct GraphSelectState {
    name_filter: String,
    /// Whether base graphs that are not demos are shown.
    show_base: bool,
    /// Whether demo graphs are shown, including base demos.
    show_demo: bool,
}

impl Default for GraphSelectState {
    fn default() -> Self {
        Self {
            name_filter: String::new(),
            show_base: false,
            show_demo: true,
        }
    }
}

/// Commands emitted from the `GraphSelect` widget.
#[derive(Debug, Default)]
pub struct GraphSelectResponse {
    /// Indicates the new graph button was clicked.
    pub new_graph: bool,
    /// Indicates the import button was clicked.
    pub import: bool,
    /// Indicates the export-all button was clicked.
    pub export_all: bool,
    /// A plain click while the focused head is named. Replace the focused head
    /// with this one.
    pub replaced: Option<gantz_ca::Head>,
    /// Open this head as a new tab, or focus it if already open.
    ///
    /// Emitted on ctrl+click of a head that is not open, or on a plain click
    /// while the focused head is an unnamed commit. This way clicking another
    /// head cannot silently lose an unnamed graph.
    pub opened: Option<gantz_ca::Head>,
    /// Ctrl+click on a head that is already open. Close this head.
    pub closed: Option<gantz_ca::Head>,
    /// The name mapping was removed.
    pub name_removed: Option<Name>,
    /// A session invite ticket was submitted via the join popup.
    pub join_ticket: Option<String>,
}

/// A row of the list of graphs.
enum Row<'a> {
    /// A named graph. `base` marks a base graph.
    Named {
        name: &'a Name,
        ca: gantz_ca::CommitAddr,
        base: bool,
    },
    /// An open graph with no name.
    Unnamed {
        ca: gantz_ca::CommitAddr,
        timestamp: &'a gantz_ca::Timestamp,
    },
}

impl<'a> GraphSelect<'a> {
    pub fn new(
        registry: &'a crate::Env<'a>,
        heads: &'a [gantz_ca::Head],
        base_names: &'a crate::reg::Names,
    ) -> Self {
        let id = egui::Id::new("gantz-graph-select");
        Self {
            registry,
            heads,
            id,
            focused_head: None,
            base_names,
            collab: None,
            clipboard: None,
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

    /// Provide the collaborative-session display state. Without this call the
    /// join button and per-row session dots are hidden.
    pub fn collab(mut self, collab: Option<&'a crate::collab::CollabUiState>) -> Self {
        self.collab = collab;
        self
    }

    /// Provide a clipboard reader for the join popup's right-click paste.
    /// Without it the paste menu item is hidden. Ctrl+V keeps working through
    /// egui's own event path.
    pub fn clipboard(mut self, clipboard: Option<&'a dyn Fn() -> Option<String>>) -> Self {
        self.clipboard = clipboard;
        self
    }

    pub fn show(&mut self, ui: &mut egui::Ui) -> GraphSelectResponse {
        let state_id = self.id.with("state");
        let mut state = ui
            .memory_mut(|mem| mem.data.get_temp::<GraphSelectState>(state_id))
            .unwrap_or_default();

        let mut response = GraphSelectResponse::default();

        ui.add_space(SECTION_GAP);
        // A name filter text field, with a filter-options button on the right
        // that opens a menu of `base`/`demo` visibility checkboxes.
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let h = ui.spacing().interact_size.y;
                let btn = ui
                    .add_sized([h, h], egui::Button::new(FILTER_GLYPH))
                    .on_hover_text("filter options");
                egui::Popup::menu(&btn)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                        ui.checkbox(&mut state.show_base, "base").on_hover_text(
                            "show base nodes, the pre-composed nodes provided with gantz",
                        );
                        ui.checkbox(&mut state.show_demo, "demo")
                            .on_hover_text("show demos");
                    });
                // The name filter fills the remaining width.
                super::kit::field_visuals(ui);
                egui::TextEdit::singleline(&mut state.name_filter)
                    .desired_width(ui.available_width())
                    .hint_text("filter graphs")
                    .margin(super::kit::FIELD_MARGIN)
                    .show(ui);
            });
        });

        // Captured by `show_named` to surface each named graph's description and
        // input/output docs on hover.
        let registry = self.registry;

        // The buttons take their own height at the bottom, and the list fills
        // the height above them.
        egui::Panel::bottom(self.id.with("footer"))
            .frame(egui::Frame::NONE)
            .show_separator_line(false)
            .resizable(false)
            .show_inside(ui, |ui| {
                ui.add_space(SECTION_GAP);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button("\u{2B07}")
                            .on_hover_text("export all named graphs")
                            .clicked()
                        {
                            response.export_all = true;
                        }
                        if ui
                            .button("\u{2B06}")
                            .on_hover_text("import graph(s)")
                            .clicked()
                        {
                            response.import = true;
                        }
                        // Join a session from an invite ticket. Only shown when a
                        // collab layer is wired.
                        if self.collab.is_some() {
                            let btn = ui
                                .button("\u{1F310} join")
                                .on_hover_text("join a session from an invite ticket");
                            let ticket_id = self.id.with("join_ticket");
                            let clipboard = self.clipboard;
                            egui::Popup::menu(&btn)
                                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                                .show(|ui| {
                                    let mut ticket = ui
                                        .data(|d| d.get_temp::<String>(ticket_id))
                                        .unwrap_or_default();
                                    ui.horizontal(|ui| {
                                        let edit = ui.add(
                                            egui::TextEdit::singleline(&mut ticket)
                                                .hint_text("paste an invite ticket"),
                                        );
                                        // Pasting is how this field is nearly always
                                        // filled. egui alone cannot read the clipboard,
                                        // so right-click paste needs a host-provided
                                        // reader. Ctrl+V works through egui's event
                                        // path regardless.
                                        if let Some(read) = clipboard {
                                            edit.context_menu(|ui| {
                                                if ui.button("paste").clicked() {
                                                    if let Some(text) = read() {
                                                        ticket = text.trim().to_string();
                                                    }
                                                    ui.close();
                                                }
                                            });
                                        }
                                        let ready = !ticket.trim().is_empty();
                                        if ui
                                            .add_enabled(ready, egui::Button::new("connect"))
                                            .clicked()
                                        {
                                            response.join_ticket = Some(ticket.trim().to_string());
                                            ticket.clear();
                                            ui.close();
                                        }
                                    });
                                    ui.data_mut(|d| d.insert_temp(ticket_id, ticket));
                                });
                        }
                        // The vault link's state, while this device is linked. A mark
                        // flags a link that only the user can fix.
                        if let Some(vault) = self.collab.and_then(|c| c.vault.as_ref()) {
                            let color = crate::style::color(ui, vault.state.role());
                            let dot = crate::widget::status_dot(ui, color);
                            let dot = match vault.state.needs_action() {
                                true => {
                                    dot | ui.label(egui::RichText::new("!").strong().color(color))
                                }
                                false => dot,
                            };
                            dot.on_hover_text(vault.hover_text());
                        }
                        // The "+" button fills the remaining width.
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            let width = egui::vec2(ui.available_width(), 0.0);
                            if ui
                                .add(egui::Button::new("+").min_size(width))
                                .on_hover_text("add graph (Ctrl+N)")
                                .clicked()
                            {
                                response.new_graph = true;
                            }
                        });
                    });
                });
            });

        ui.add_space(SECTION_GAP);
        // List all the graphs, named then unnamed. Only the rows in view are
        // built, as a store can hold many graphs.
        let rows = rows(self.registry.registry, self.base_names, self.heads, &state);
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = ROW_GAP;
            let collab = self.collab;

            // Show a named graph row and its right-click menu, and handle
            // clicks.
            let show_named = |ui: &mut egui::Ui,
                              name: &Name,
                              ca: &gantz_ca::CommitAddr,
                              base: bool,
                              heads: &[gantz_ca::Head],
                              focused_head: Option<usize>,
                              response: &mut GraphSelectResponse| {
                let name_str = name.to_string();
                let row_type = if base {
                    HeadRowType::Base(&name_str)
                } else {
                    HeadRowType::Named(&name_str)
                };
                let head = gantz_ca::Head::Branch(name.clone());
                // A live session's dot beside the name, mirroring the
                // graph tabs. Sessions outlive their tabs, so this is
                // where a closed head's session stays visible.
                let status = collab.and_then(|c| c.sessions.get(name)).map(|d| {
                    (
                        crate::style::color(ui, d.conn.role()),
                        d.hover_text().into(),
                    )
                });
                let mut res = head_row(heads, &head, row_type, ca, focused_head, status, ui);
                // Show the graph's description and socket docs on hover.
                res.row = res.row.on_hover_ui(|ui| {
                    // Re-assert wrap width every frame. See `show_socket_doc`.
                    let max_width = ui.spacing().tooltip_width;
                    ui.set_max_width(max_width);
                    crate::node_info_ui(&registry.command_info(&name_str), ui);
                });
                // Only named non-base rows offer an `×` and are deletable.
                let deletable = res.delete.is_some();
                res.row.context_menu(|ui| {
                    if ui.button("open").clicked() {
                        response.replaced = Some(head.clone());
                        ui.close();
                    }
                    if ui.button("open tab").clicked() {
                        response.opened = Some(head.clone());
                        ui.close();
                    }
                    // The associated demo to offer, if any.
                    if let Some(demo_name) = &registry.demo_graph(&name_str) {
                        if ui
                            .button("demo")
                            .on_hover_text("open the associated demo in a new tab")
                            .clicked()
                        {
                            response.opened = Some(gantz_ca::Head::Branch(
                                demo_name.parse().expect("infallible"),
                            ));
                            ui.close();
                        }
                    }
                    if deletable && ui.button("delete").clicked() {
                        response.name_removed = Some(name.clone());
                        ui.close();
                    }
                });
                if res.row.clicked() {
                    click_head(ui, heads, focused_head, head, response);
                } else if let Some(delete) = res.delete {
                    if delete.clicked() {
                        response.name_removed = Some(name.clone());
                    }
                }
            };

            for row in &rows {
                let row_id = match row {
                    Row::Named { name, .. } => self.id.with(("named", name)),
                    Row::Unnamed { ca, .. } => self.id.with(("unnamed", ca)),
                };
                super::kit::lazy_item(ui, row_id, |ui| match *row {
                    Row::Named { name, ca, base } => show_named(
                        ui,
                        name,
                        &ca,
                        base,
                        self.heads,
                        self.focused_head,
                        &mut response,
                    ),
                    Row::Unnamed { ca, timestamp } => {
                        let head = gantz_ca::Head::Commit(ca);
                        let row_type = HeadRowType::Unnamed(timestamp);
                        let res = head_row(
                            self.heads,
                            &head,
                            row_type,
                            &ca,
                            self.focused_head,
                            None,
                            ui,
                        );
                        if res.row.clicked() {
                            click_head(ui, self.heads, self.focused_head, head, &mut response);
                        }
                    }
                });
            }
        });

        ui.memory_mut(|mem| mem.data.insert_temp(state_id, state));

        response
    }
}

/// All commits in the registry, sorted newest to oldest.
///
/// The head-listing widgets share this ordering for their unnamed-commit
/// rows.
pub fn commits_by_recency(
    reg: &gantz_ca::Registry,
) -> Vec<(&gantz_ca::CommitAddr, &gantz_ca::Commit)> {
    let mut commits: Vec<_> = reg.commits().iter().collect();
    commits.sort_by(|(_, a), (_, b)| b.timestamp.cmp(&a.timestamp));
    commits
}

/// The rows of the list of graphs in the order they show.
///
/// Names fall into three groups. User-named graphs that are not demos come
/// first, then base graphs that are not demos, then all demos. The open graphs
/// with no name follow, newest first. A nested `parent:child` graph is hidden,
/// as it is reached through its parent.
fn rows<'a>(
    registry: &'a gantz_ca::Registry,
    base_names: &crate::reg::Names,
    heads: &[gantz_ca::Head],
    state: &GraphSelectState,
) -> Vec<Row<'a>> {
    let matches = |text: String| {
        state
            .name_filter
            .split_whitespace()
            .all(|s| text.contains(s))
    };
    let unfiltered = state.name_filter.is_empty();
    let named: Vec<(&Name, gantz_ca::CommitAddr)> = registry
        .heads()
        .filter(|(name, _)| !name.is_nested() && (unfiltered || matches(name.to_string())))
        .collect();
    let is_base = |name: &Name| base_names.contains_key(name);
    let row = |&(name, ca): &(&'a Name, gantz_ca::CommitAddr)| Row::Named {
        name,
        ca,
        base: is_base(name),
    };
    let user = named.iter().filter(|(n, _)| !is_base(n) && !is_demo(n));
    let base = named
        .iter()
        .filter(|(n, _)| state.show_base && is_base(n) && !is_demo(n));
    let demo = named.iter().filter(|(n, _)| state.show_demo && is_demo(n));
    let mut rows: Vec<Row<'a>> = user.chain(base).chain(demo).map(row).collect();

    // An open commit with a name already has its row.
    let shown: HashSet<gantz_ca::CommitAddr> = rows
        .iter()
        .filter_map(|row| match row {
            Row::Named { ca, .. } => Some(*ca),
            Row::Unnamed { .. } => None,
        })
        .collect();
    let mut unnamed: Vec<(gantz_ca::CommitAddr, &gantz_ca::Timestamp)> = heads
        .iter()
        .filter_map(|head| match head {
            gantz_ca::Head::Commit(ca) if !shown.contains(ca) => Some(*ca),
            _ => None,
        })
        .filter(|ca| unfiltered || matches(ca.to_string()))
        .filter_map(|ca| Some((ca, &registry.commits().get(&ca)?.timestamp)))
        .collect();
    unnamed.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(&b.0)));
    unnamed.dedup_by_key(|(ca, _)| *ca);
    rows.extend(
        unnamed
            .into_iter()
            .map(|(ca, timestamp)| Row::Unnamed { ca, timestamp }),
    );
    rows
}

/// Whether the name is in the `demo.` namespace of demo graphs.
pub fn is_demo(name: &Name) -> bool {
    name.segments()
        .first()
        .is_some_and(|s| s.starts_with("demo."))
}

/// Update `response` for a click on the row for `head`.
///
/// Ctrl+click toggles the head. It closes the head if open, otherwise opens it
/// as a new tab. A plain click replaces the focused head. When the focused
/// head is an unnamed commit, the clicked head opens as a new tab instead, or
/// is focused if already open. The unnamed graph is then not lost.
pub(crate) fn click_head(
    ui: &egui::Ui,
    heads: &[gantz_ca::Head],
    focused_head: Option<usize>,
    head: gantz_ca::Head,
    response: &mut GraphSelectResponse,
) {
    let ctrl = ui.input(|i| i.modifiers.ctrl);
    if ctrl {
        if heads.contains(&head) {
            response.closed = Some(head);
        } else {
            response.opened = Some(head);
        }
        return;
    }
    let focused_is_named = focused_head
        .and_then(|ix| heads.get(ix))
        .is_some_and(|head| matches!(head, gantz_ca::Head::Branch(_)));
    if focused_is_named {
        response.replaced = Some(head);
    } else {
        response.opened = Some(head);
    }
}
