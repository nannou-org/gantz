//! A simple widget for selecting between, naming and creating new graphs.

use super::head_row::{HeadRowType, head_row};
use gantz_ca::Name;
use std::collections::HashSet;

/// The glyph for the filter-options button (swap if it doesn't render).
const FILTER_GLYPH: &str = "⛭";

/// A widget for selecting between, naming, and creating new graphs.
pub struct GraphSelect<'a> {
    id: egui::Id,
    registry: &'a crate::Env<'a>,
    heads: &'a [gantz_ca::Head],
    focused_head: Option<usize>,
    base_names: &'a crate::reg::Names,
}

#[derive(Clone)]
struct GraphSelectState {
    name_filter: String,
    /// Whether base (non-demo) graphs are shown.
    show_base: bool,
    /// Whether demo graphs are shown (including base demos).
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
    /// Click while the focused head is named: replace the focused head with this one.
    pub replaced: Option<gantz_ca::Head>,
    /// Open this head as a new tab, or focus it if already open.
    ///
    /// Emitted on ctrl+click of a head that is not open, or on a plain click
    /// while the focused head is an unnamed commit (so that clicking another
    /// head can't silently lose an unnamed graph).
    pub opened: Option<gantz_ca::Head>,
    /// Ctrl+click on a head that is already open: close this head.
    pub closed: Option<gantz_ca::Head>,
    /// The name mapping was removed.
    pub name_removed: Option<Name>,
}

impl GraphSelectResponse {
    /// Combine two responses, preferring `Some` values from `other`.
    pub fn union(self, other: Self) -> Self {
        Self {
            new_graph: self.new_graph || other.new_graph,
            import: self.import || other.import,
            export_all: self.export_all || other.export_all,
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
                egui::TextEdit::singleline(&mut state.name_filter)
                    .desired_width(ui.available_width())
                    .hint_text("🔎 Name Filter")
                    .show(ui);
            });
        });

        let names = crate::reg::names(self.registry.registry);
        // Captured by `show_named` to surface each named graph's description and
        // input/output docs on hover.
        let registry = self.registry;

        // List all the graphs, named then unnamed.
        egui::ScrollArea::vertical()
            // Limit the scroll height to allow for the `+` button below.
            .max_height(
                ui.available_height() - ui.spacing().interact_size.y - ui.spacing().item_spacing.y,
            )
            .show(ui, |ui| {
                // Partition names into groups:
                // 1. User-named, non-demo
                // 2. Base-named, non-demo
                // 3. All demos (alphabetical, regardless of user/base)
                let is_base = |name: &Name| self.base_names.contains_key(name);
                let is_demo = self::is_demo;
                // Nested graphs (`parent:child`) are hidden from the root list;
                // they are reached by navigating into their parent.
                let is_nested = |name: &Name| name.is_nested();
                let matches_filter = |name: &str| {
                    state.name_filter.is_empty()
                        || state
                            .name_filter
                            .split_whitespace()
                            .all(|s| name.contains(s))
                };

                let mut visited = HashSet::new();

                // Helper: show a named graph row, its right-click menu, and
                // handle clicks.
                let show_named =
                    |ui: &mut egui::Ui,
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
                        let mut res = head_row(heads, &head, row_type, ca, focused_head, ui);
                        // Show the graph's description + input/output docs on hover.
                        res.row = res.row.on_hover_ui(|ui| {
                            // Re-assert wrap width every frame (see `socket_hover`).
                            let max_width = ui.spacing().tooltip_width;
                            ui.set_max_width(max_width);
                            crate::node_info_ui(&registry.command_info(&name_str), ui);
                        });
                        // Deletable iff the row offers an `×` (named, non-base).
                        let deletable = res.delete.is_some();
                        // The associated demo to offer, if any.
                        let demo = registry.demo_graph(&name_str);
                        res.row.context_menu(|ui| {
                            if ui.button("open").clicked() {
                                response.replaced = Some(head.clone());
                                ui.close();
                            }
                            if ui.button("open tab").clicked() {
                                response.opened = Some(head.clone());
                                ui.close();
                            }
                            if let Some(demo_name) = &demo {
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

                // 1. User-named, non-demo.
                for (name, ca) in names
                    .iter()
                    .filter(|(n, _)| !is_base(n) && !is_demo(n) && !is_nested(n))
                {
                    if !matches_filter(&name.to_string()) {
                        continue;
                    }
                    visited.insert(*ca);
                    show_named(
                        ui,
                        name,
                        ca,
                        false,
                        self.heads,
                        self.focused_head,
                        &mut response,
                    );
                }

                // 2. Base-named, non-demo (hidden when the `base` filter is off).
                for (name, ca) in names
                    .iter()
                    .filter(|(n, _)| state.show_base && is_base(n) && !is_demo(n) && !is_nested(n))
                {
                    if !matches_filter(&name.to_string()) {
                        continue;
                    }
                    visited.insert(*ca);
                    show_named(
                        ui,
                        name,
                        ca,
                        true,
                        self.heads,
                        self.focused_head,
                        &mut response,
                    );
                }

                // 3. All demos, alphabetical, regardless of user/base (hidden
                //    when the `demo` filter is off; shown even if also a base).
                for (name, ca) in names
                    .iter()
                    .filter(|(n, _)| state.show_demo && is_demo(n) && !is_nested(n))
                {
                    if !matches_filter(&name.to_string()) {
                        continue;
                    }
                    visited.insert(*ca);
                    show_named(
                        ui,
                        name,
                        ca,
                        is_base(name),
                        self.heads,
                        self.focused_head,
                        &mut response,
                    );
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
                for (ca, commit) in commits_by_recency(self.registry.registry)
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
                        click_head(ui, self.heads, self.focused_head, head, &mut response);
                    }
                }
            });

        ui.horizontal(|ui| {
            // Place import and export buttons on the right.
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
                // Fill remaining space with the "+" button.
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    if ui
                        .add(egui::Button::new("+").min_size(ui.available_size()))
                        .on_hover_text("add graph (Ctrl+N)")
                        .clicked()
                    {
                        response.new_graph = true;
                    }
                });
            });
        });

        // Store the modified state back in memory
        ui.memory_mut(|mem| mem.data.insert_temp(state_id, state));

        response
    }
}

/// All commits in the registry, sorted newest to oldest.
///
/// The head-listing widgets (graph select, history view) share this ordering
/// for their unnamed-commit rows.
pub fn commits_by_recency(
    reg: &gantz_ca::Registry,
) -> Vec<(&gantz_ca::CommitAddr, &gantz_ca::Commit)> {
    let mut commits: Vec<_> = reg.commits().iter().collect();
    commits.sort_by(|(_, a), (_, b)| b.timestamp.cmp(&a.timestamp));
    commits
}

/// Whether the name follows the `demo-*` naming convention for demo graphs.
pub(crate) fn is_demo(name: &Name) -> bool {
    name.segments()
        .first()
        .is_some_and(|s| s.starts_with("demo-"))
}

/// Update `response` for a click on the row for `head`.
///
/// Ctrl+click toggles the head: closes it if open, otherwise opens it as a new
/// tab. A plain click replaces the focused head, unless the focused head is an
/// unnamed commit, in which case the clicked head is opened as a new tab
/// instead (or focused if already open) so that the unnamed graph isn't lost.
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
