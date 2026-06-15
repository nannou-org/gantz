//! A validated name-editing `TextEdit` for a head.

/// Response from [`head_name_edit`].
pub struct HeadNameEditResponse {
    /// The inner `TextEdit` response.
    pub response: egui::Response,
    /// A valid rename committed via Enter or focus loss.
    pub new_branch: Option<(gantz_ca::Head, String)>,
}

/// Show a validated name-editing `TextEdit` for the given head.
///
/// Text is red when the name is empty, already exists (excluding the head's own
/// current name), or contains the reserved nested-graph separator `:`. Returns
/// a valid `new_branch` when a rename is committed via Enter or focus loss.
///
/// `:` is reserved for the `parent:child` nesting convention and is only ever
/// produced by creating a nested graph - so renaming always targets a plain
/// (root) name. For a nested graph, that effectively saves it as a new root
/// graph copy.
pub fn head_name_edit(
    head: &gantz_ca::Head,
    name: &mut String,
    names: &gantz_ca::registry::Names,
    ui: &mut egui::Ui,
) -> HeadNameEditResponse {
    let name_exists = names.contains_key(name.as_str());
    let is_current_name = matches!(head, gantz_ca::Head::Branch(n) if n == name);
    let is_empty = name.is_empty();
    let has_separator = name.contains(crate::node::NESTED_SEP);
    let is_invalid = is_empty || (!is_current_name && (name_exists || has_separator));

    let text_color = if is_invalid && !is_current_name {
        egui::Color32::RED
    } else {
        ui.visuals().text_color()
    };

    let text_edit = egui::TextEdit::singleline(name)
        .desired_width(ui.available_width())
        .text_color(text_color)
        .hint_text("name");
    let mut response = ui.add(text_edit);
    if has_separator && !is_current_name {
        response = response.on_hover_text(format!(
            "'{}' is reserved for nested graphs; use a plain name",
            crate::node::NESTED_SEP,
        ));
    }

    let enter_pressed = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let focus_lost = response.lost_focus() && !ui.input(|i| i.key_pressed(egui::Key::Escape));
    let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));

    let mut new_branch = None;
    if enter_pressed || focus_lost {
        if !is_empty && !is_invalid {
            new_branch = Some((head.clone(), name.clone()));
        }
        *name = head_name(head);
    } else if cancelled {
        *name = head_name(head);
    }

    HeadNameEditResponse {
        response,
        new_branch,
    }
}

pub fn head_name(head: &gantz_ca::Head) -> String {
    match head {
        gantz_ca::Head::Branch(n) => n.clone(),
        gantz_ca::Head::Commit(_) => String::new(),
    }
}
