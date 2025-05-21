/// A toggle widget that behaves like a label but can be toggled on/off.
pub struct LabelToggle<'a> {
    /// Either a user-provided Label or text that will be converted to a Label
    label: egui::Label,
    /// The toggle state to modify
    selected: &'a mut bool,
}

impl<'a> LabelToggle<'a> {
    /// Create a new LabelToggle from raw text
    pub fn new(text: impl Into<egui::WidgetText>, selected: &'a mut bool) -> Self {
        Self {
            label: egui::Label::new(text),
            selected,
        }
    }

    /// Create a LabelToggle from an existing Label
    /// This allows using all Label options like wrap(), truncate(), etc.
    pub fn from_label(label: egui::Label, selected: &'a mut bool) -> Self {
        Self { label, selected }
    }
}

impl<'a> egui::Widget for LabelToggle<'a> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let (galley_pos, galley, mut response) = self.label.layout_in_ui(ui);

        response = response.union(ui.interact(response.rect, response.id, egui::Sense::click()));

        if response.clicked() {
            *self.selected = !*self.selected;
            response.mark_changed();
        }

        let text_color = if response.hovered() {
            ui.visuals().strong_text_color()
        } else if *self.selected {
            ui.visuals().selection.stroke.color
        } else {
            ui.visuals().weak_text_color()
        };

        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                ui.is_enabled(),
                *self.selected,
                galley.text(),
            )
        });

        ui.painter()
            .add(egui::epaint::TextShape::new(galley_pos, galley, text_color));

        response
    }
}
