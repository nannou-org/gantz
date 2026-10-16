//! A text edit over a value owned elsewhere, committed on focus loss.

/// Response from [`buffered_edit`].
pub struct BufferedEditResponse {
    /// The inner `TextEdit` response.
    pub response: egui::Response,
    /// The text, when the field lost focus with an edit to commit.
    pub committed: Option<String>,
}

/// The text being edited, plus the source value it was seeded from. It is kept
/// in egui temp memory only while the field has focus.
#[derive(Clone, Debug, PartialEq)]
struct Buffer {
    seed: String,
    text: String,
}

impl Buffer {
    fn new(source: &str) -> Self {
        Self {
            seed: source.to_string(),
            text: source.to_string(),
        }
    }

    /// The text to commit, if it differs from the text the edit started from.
    fn edit(&self) -> Option<&str> {
        (self.text != self.seed).then_some(self.text.as_str())
    }
}

/// Show a text edit over `source`, a value owned elsewhere such as a graph's
/// description in the registry.
///
/// Without focus the field shows `source` as it is. With focus it shows a
/// buffer that follows `source` until the user edits it. Focus loss commits
/// the edit. A focus and blur with no edit commits nothing, so the field never
/// writes its stale text over a change to `source` from elsewhere.
///
/// `id` names the field. `text_edit` configures the `TextEdit` over the
/// buffer.
pub fn buffered_edit(
    ui: &mut egui::Ui,
    id: egui::Id,
    source: &str,
    text_edit: impl FnOnce(&mut String) -> egui::TextEdit<'_>,
) -> BufferedEditResponse {
    // A widget drawn earlier in the pass can take the focus before the field
    // draws. The field must still see its buffer then, to commit it.
    let active = ui.memory(|m| m.has_focus(id) || m.had_focus_last_frame(id));
    let prev = ui.data(|d| d.get_temp::<Buffer>(id));
    let mut buffer = follow(prev, source, active);
    let response = ui.add(text_edit(&mut buffer.text).id(id));
    let committed = response
        .lost_focus()
        .then(|| buffer.edit().map(str::to_string))
        .flatten();
    if response.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, buffer));
    } else {
        ui.data_mut(|d| d.remove::<Buffer>(id));
    }
    BufferedEditResponse {
        response,
        committed,
    }
}

/// This frame's buffer, from the last frame's `prev` buffer.
///
/// An inactive field, one without focus this frame or last frame, starts again
/// from `source`. An active field keeps its buffer while `source` is unchanged
/// or the user has edited the text, and otherwise follows `source`.
fn follow(prev: Option<Buffer>, source: &str, active: bool) -> Buffer {
    match prev {
        Some(b) if active && (b.seed == source || b.edit().is_some()) => b,
        _ => Buffer::new(source),
    }
}

#[cfg(test)]
mod tests {
    use super::{Buffer, follow};

    fn buffer(seed: &str, text: &str) -> Buffer {
        Buffer {
            seed: seed.to_string(),
            text: text.to_string(),
        }
    }

    #[test]
    fn an_inactive_field_shows_the_source() {
        assert_eq!(follow(None, "a", false), buffer("a", "a"));
        assert_eq!(
            follow(Some(buffer("a", "ab")), "a", false),
            buffer("a", "a")
        );
        assert_eq!(follow(Some(buffer("a", "a")), "b", false), buffer("b", "b"));
    }

    #[test]
    fn an_active_field_keeps_its_edit() {
        assert_eq!(
            follow(Some(buffer("a", "ab")), "a", true),
            buffer("a", "ab")
        );
        // The user's edit wins over a change from elsewhere.
        assert_eq!(
            follow(Some(buffer("a", "ab")), "c", true),
            buffer("a", "ab")
        );
    }

    #[test]
    fn an_unedited_active_field_follows_the_source() {
        assert_eq!(follow(Some(buffer("a", "a")), "b", true), buffer("b", "b"));
        assert_eq!(follow(Some(buffer("b", "b")), "b", true), buffer("b", "b"));
    }

    #[test]
    fn only_an_edit_commits() {
        assert_eq!(buffer("a", "a").edit(), None);
        assert_eq!(buffer("a", "ab").edit(), Some("ab"));
        assert_eq!(buffer("a", "").edit(), Some(""));
    }

    const FIELD: &str = "field";

    /// How a pass treats the field's focus.
    enum Focus {
        Keep,
        /// The user presses Enter, which blurs a single-line field.
        Enter,
        /// A widget drawn before the field takes focus.
        Taken,
    }

    /// Run one pass of a single-line field over `source`, returning its commit.
    fn pass(ctx: &egui::Context, source: &str, focus: Focus) -> Option<String> {
        let id = egui::Id::new(FIELD);
        let events = match focus {
            Focus::Enter => vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            Focus::Keep | Focus::Taken => vec![],
        };
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut committed = None;
        let _ = ctx.run_ui(input, |ui| {
            if let Focus::Taken = focus {
                ui.memory_mut(|m| m.request_focus(egui::Id::new("other")));
            }
            let edit = super::buffered_edit(ui, id, source, |t| egui::TextEdit::singleline(t));
            committed = edit.committed;
        });
        committed
    }

    /// Show the field once, then give it focus.
    fn focused_field() -> egui::Context {
        let ctx = egui::Context::default();
        pass(&ctx, "a", Focus::Keep);
        ctx.memory_mut(|m| m.request_focus(egui::Id::new(FIELD)));
        assert_eq!(pass(&ctx, "a", Focus::Keep), None);
        ctx
    }

    fn stored(ctx: &egui::Context) -> Option<Buffer> {
        ctx.data(|d| d.get_temp::<Buffer>(egui::Id::new(FIELD)))
    }

    /// A change from elsewhere shows in a focused field that has no edit, and
    /// a blur then commits nothing.
    #[test]
    fn a_focus_and_blur_keeps_a_change_from_elsewhere() {
        let ctx = focused_field();
        assert_eq!(stored(&ctx), Some(buffer("a", "a")));
        assert_eq!(pass(&ctx, "b", Focus::Keep), None);
        assert_eq!(stored(&ctx), Some(buffer("b", "b")));
        assert_eq!(pass(&ctx, "b", Focus::Enter), None);
        assert_eq!(stored(&ctx), None);
    }

    /// A blur after an edit commits the edited text once.
    #[test]
    fn a_blur_commits_an_edit() {
        let ctx = focused_field();
        let id = egui::Id::new(FIELD);
        ctx.data_mut(|d| d.insert_temp(id, buffer("a", "ab")));
        assert_eq!(pass(&ctx, "a", Focus::Enter), Some("ab".to_string()));
        assert_eq!(pass(&ctx, "a", Focus::Keep), None);
    }

    /// Focus taken by another widget earlier in the pass still commits the
    /// edit, though the field no longer has focus when it draws.
    #[test]
    fn focus_taken_earlier_in_the_pass_commits_an_edit() {
        let ctx = focused_field();
        let id = egui::Id::new(FIELD);
        ctx.data_mut(|d| d.insert_temp(id, buffer("a", "ab")));
        assert_eq!(pass(&ctx, "a", Focus::Taken), Some("ab".to_string()));
        assert_eq!(stored(&ctx), None);
    }
}
