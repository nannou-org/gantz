//! A checkbox that enables/disables an adjacent widget.

/// A checkbox on the left enabling/disabling a widget on the right.
///
/// The checkbox is bound to `on`; the wrapped `widget` is shown enabled when
/// `on` is true and disabled (greyed, non-interactive) when false. The gap
/// between the two is tightened so the pair reads as one control, and an
/// optional fixed [`width`][Self::width] keeps a following column aligned as the
/// widget's content width changes.
///
/// The returned [`Response`](egui::Response) is the union of the checkbox and
/// the inner widget, so `changed()` is true when either the toggle flips or the
/// inner widget is edited.
pub struct CheckboxEnabled<'a, W> {
    on: &'a mut bool,
    widget: W,
    width: Option<f32>,
}

impl<'a, W: egui::Widget> CheckboxEnabled<'a, W> {
    /// Wrap `widget` with a checkbox bound to `on`.
    pub fn new(on: &'a mut bool, widget: W) -> Self {
        Self {
            on,
            widget,
            width: None,
        }
    }

    /// Give the wrapped widget a fixed width, so neighbouring widgets stay put
    /// as its content width changes.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
}

impl<W: egui::Widget> egui::Widget for CheckboxEnabled<'_, W> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let Self { on, widget, width } = self;
        ui.horizontal(|ui| {
            // Tighten the gap so the checkbox reads as part of its widget.
            ui.spacing_mut().item_spacing.x *= 0.25;
            let checkbox = ui.checkbox(on, "");
            let enabled = *on;
            let inner = ui
                .add_enabled_ui(enabled, |ui| match width {
                    Some(w) => ui.add_sized([w, ui.spacing().interact_size.y], widget),
                    None => ui.add(widget),
                })
                .inner;
            checkbox.union(inner)
        })
        .inner
    }
}
