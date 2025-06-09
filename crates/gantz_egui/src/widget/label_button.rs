/// A button widget that visually looks like a label.
pub struct LabelButton {
    /// Either a user-provided Label or text that will be converted to a Label
    label: egui::Label,
}

impl LabelButton {
    /// Create a new LabelButton from raw text
    pub fn new(text: impl Into<egui::WidgetText>) -> Self {
        Self {
            label: egui::Label::new(text),
        }
    }

    /// Create a LabelButton from an existing Label
    /// This allows using all Label options like wrap(), truncate(), etc.
    pub fn from_label(label: egui::Label) -> Self {
        Self { label }
    }
}

impl egui::Widget for LabelButton {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let (galley_pos, galley, mut response) = self.label.layout_in_ui(ui);

        response = response.union(ui.interact(response.rect, response.id, egui::Sense::click()));

        let text_color = if response.hovered() {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().weak_text_color()
        };

        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), galley.text())
        });

        ui.painter()
            .add(egui::epaint::TextShape::new(galley_pos, galley, text_color));

        response
    }
}
