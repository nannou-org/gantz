/// A toggle widget that behaves like a label but can be toggled on/off.
///
/// By default the text is dim when off, the accent (selection) colour when on,
/// and the strong text colour when hovered. Each of these can be overridden via
/// [`default_color`][Self::default_color], [`selected_color`][Self::selected_color]
/// and [`hovered_color`][Self::hovered_color].
pub struct LabelToggle<'a> {
    /// Either a user-provided Label or text that will be converted to a Label
    label: egui::Label,
    /// The toggle state to modify
    selected: &'a mut bool,
    /// Colour when off (unselected, not hovered). Defaults to `weak_text_color`.
    default_color: Option<egui::Color32>,
    /// Colour when on (selected, not hovered). Defaults to the selection colour.
    selected_color: Option<egui::Color32>,
    /// Colour when hovered. Defaults to `strong_text_color`.
    hovered_color: Option<egui::Color32>,
}

impl<'a> LabelToggle<'a> {
    /// Create a new LabelToggle from raw text
    pub fn new(text: impl Into<egui::WidgetText>, selected: &'a mut bool) -> Self {
        Self::from_label(egui::Label::new(text), selected)
    }

    /// Create a LabelToggle from an existing Label
    /// This allows using all Label options like wrap(), truncate(), etc.
    pub fn from_label(label: egui::Label, selected: &'a mut bool) -> Self {
        Self {
            label,
            selected,
            default_color: None,
            selected_color: None,
            hovered_color: None,
        }
    }

    /// Override the colour shown when off (unselected and not hovered).
    pub fn default_color(mut self, color: egui::Color32) -> Self {
        self.default_color = Some(color);
        self
    }

    /// Override the colour shown when on (selected and not hovered).
    pub fn selected_color(mut self, color: egui::Color32) -> Self {
        self.selected_color = Some(color);
        self
    }

    /// Override the colour shown when hovered.
    pub fn hovered_color(mut self, color: egui::Color32) -> Self {
        self.hovered_color = Some(color);
        self
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
            self.hovered_color
                .unwrap_or_else(|| ui.visuals().strong_text_color())
        } else if *self.selected {
            self.selected_color
                .unwrap_or_else(|| ui.visuals().selection.stroke.color)
        } else {
            self.default_color
                .unwrap_or_else(|| ui.visuals().weak_text_color())
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
