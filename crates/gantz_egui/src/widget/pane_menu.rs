use super::{LabelToggle, gantz::ViewToggles};

/// A menu widget for toggling pane visibility.
///
/// Displays as a subtle icon in the bottom-right corner. When clicked, a
/// separate menu window appears above with toggle options for each pane.
pub struct PaneMenu<'a> {
    view_toggles: &'a mut ViewToggles,
}

/// Persistent state for the pane menu.
#[derive(Clone, Default)]
struct PaneMenuState {
    open: bool,
}

impl<'a> PaneMenu<'a> {
    pub fn new(view_toggles: &'a mut ViewToggles) -> Self {
        Self { view_toggles }
    }

    /// Show the pane menu anchored to the given position (bottom-right corner).
    pub fn show(self, ctx: &egui::Context, anchor_pos: egui::Pos2) {
        let id = egui::Id::new("pane_menu");
        let state_id = id.with("state");

        // Load state from memory.
        let mut state: PaneMenuState = ctx
            .memory(|m| m.data.get_temp(state_id))
            .unwrap_or_default();

        // Animate the open/close transition.
        let animation_time = 0.15;
        let openness = ctx.animate_bool_with_time(id.with("openness"), state.open, animation_time);

        // The menu button window (fixed at bottom-right, transparent).
        let button_response = egui::Area::new(id.with("button"))
            .pivot(egui::Align2::RIGHT_BOTTOM)
            .fixed_pos(anchor_pos)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::NONE.show(ui, |ui| {
                    let icon_text = egui::RichText::new("👁").size(24.0);
                    let response = ui.add(LabelToggle::new(icon_text, &mut state.open));

                    if !state.open {
                        response.on_hover_text("Toggle pane visibility")
                    } else {
                        response
                    }
                })
            });

        // The menu items window (separate, transparent, positioned above button).
        let menu_items_response = if openness > 0.0 {
            // Position the menu window above the button.
            let spacing = ctx.style().spacing.item_spacing.y;
            let menu_anchor = anchor_pos
                - egui::vec2(0.0, button_response.inner.response.rect.height() + spacing);

            let mut items_response: Option<egui::Response> = None;

            egui::Area::new(id.with("menu"))
                .pivot(egui::Align2::RIGHT_BOTTOM)
                .fixed_pos(menu_anchor)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::NONE.show(ui, |ui| {
                        ui.with_layout(egui::Layout::bottom_up(egui::Align::RIGHT), |ui| {
                            // Menu items in reverse order (bottom-up layout).
                            items_response = [
                                menu_item(ui, "Steel", &mut self.view_toggles.steel),
                                menu_item(ui, "Logs", &mut self.view_toggles.logs),
                                menu_item(
                                    ui,
                                    "Node Inspector",
                                    &mut self.view_toggles.node_inspector,
                                ),
                                menu_item(ui, "Graph Select", &mut self.view_toggles.graph_select),
                                menu_item(ui, "Graph Config", &mut self.view_toggles.graph_config),
                            ]
                            .into_iter()
                            .reduce(|acc, r| acc.union(r));
                        });
                    });
                });

            items_response
        } else {
            None
        };

        // Close menu if clicked outside both button and menu areas.
        if state.open {
            let button_interacted = button_response.inner.inner.hovered()
                || button_response.inner.inner.is_pointer_button_down_on();
            let menu_interacted = menu_items_response
                .as_ref()
                .is_some_and(|r| r.hovered() || r.is_pointer_button_down_on());

            let clicked_outside =
                ctx.input(|i| i.pointer.any_pressed()) && !button_interacted && !menu_interacted;

            if clicked_outside {
                state.open = false;
            }
        }

        // Store state back to memory.
        ctx.memory_mut(|m| m.data.insert_temp(state_id, state));
    }
}

/// Render a single menu item as a toggle.
fn menu_item(ui: &mut egui::Ui, label: &str, selected: &mut bool) -> egui::Response {
    ui.add(LabelToggle::new(label, selected))
}
