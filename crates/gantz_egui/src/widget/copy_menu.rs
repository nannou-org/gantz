//! A context menu that copies the text selected across labels.
//!
//! egui copies a label selection only on a copy event, such as Ctrl+C. A
//! secondary press on a label or a press on a menu item also drops the
//! selection. So [`keep`] holds the selection as a secondary press begins,
//! and restores it in each pass while the [`menu`] is open. The "copy" item
//! sends a copy event in the next pass, before the labels draw again.

use egui::text_selection::LabelSelectionState;

/// The state of a copy menu, in egui's temporary memory.
#[derive(Clone, Default)]
struct State {
    /// The label selection as the last secondary press began.
    kept: Option<LabelSelectionState>,
    /// The menu was open in the last pass.
    open: bool,
    /// Copy the kept selection in the next pass.
    copy: bool,
}

/// Keep the label selection for the copy menu `id`, or restore and copy a
/// kept one. Call this in each pass before the labels of the menu draw.
pub(crate) fn keep(ui: &egui::Ui, id: egui::Id) {
    let ctx = ui.ctx();
    let mut state = ctx
        .data_mut(|d| d.get_temp::<State>(id))
        .unwrap_or_default();
    let open = std::mem::take(&mut state.open);
    let copy = std::mem::take(&mut state.copy);
    if ui.input(|i| i.pointer.secondary_pressed()) {
        state.kept = ctx.with_plugin(|s: &mut LabelSelectionState| s.clone());
    } else if let Some(kept) = state.kept.as_ref().filter(|_| open || copy) {
        ctx.with_plugin(|s: &mut LabelSelectionState| *s = kept.clone());
        if copy {
            ctx.input_mut(|i| i.events.push(egui::Event::Copy));
        }
    }
    ctx.data_mut(|d| d.insert_temp(id, state));
}

/// A context menu on the label `response` with a "copy" item for the
/// selection that [`keep`] holds for `id`.
pub(crate) fn menu(response: &egui::Response, id: egui::Id) {
    response.context_menu(|ui| {
        let ctx = ui.ctx().clone();
        let mut state = ctx.data(|d| d.get_temp::<State>(id)).unwrap_or_default();
        state.open = true;
        let selected = state.kept.as_ref().is_some_and(|s| s.has_selection());
        if ui
            .add_enabled(selected, egui::Button::new("copy"))
            .clicked()
        {
            state.copy = true;
            ctx.request_repaint();
            ui.close();
        }
        ctx.data_mut(|d| d.insert_temp(id, state));
    });
}
