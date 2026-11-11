//! A read-only view of scheme text, for example compiled steel source. It
//! marks byte ranges, for example the fns emitted for the selected node or
//! diagnostic spans, and can scroll to an offset.
//!
//! The view lexes and lays out only the lines within the clip rectangle of
//! its `Ui`. Its one cost that grows with the text is a hash of the text in
//! each frame. A new text is indexed once.
//!
//! The label selection of egui needs both ends of a selection to be painted.
//! The view therefore keeps its own selection as byte offsets into the whole
//! text. A selection can then run past the lines in view.

use crate::syntax::{self, Mode, Theme};
use std::{ops::Range, sync::Arc};

/// A syntax-highlighted, read-only view of scheme text.
pub struct SteelView<'a> {
    code: &'a str,
    /// Byte ranges to emphasise, for example the spans of the selected node.
    highlights: &'a [Range<usize>],
    /// Byte ranges of diagnostic spans, tinted with the error colour.
    errors: &'a [Range<usize>],
    /// A byte offset to scroll into view this frame.
    scroll_to: Option<usize>,
}

/// Where each line of a text starts, and what the lexer is inside of there.
#[derive(Debug)]
struct Lines {
    /// The byte offset of the start of each line, then the length of the
    /// text.
    starts: Vec<usize>,
    /// The lexer mode at the start of each line.
    modes: Vec<Mode>,
    /// The length of the longest line, in characters.
    max_chars: usize,
}

/// The selected text as byte offsets. `anchor` stays where the selection
/// began and `head` follows the pointer.
#[derive(Clone, Copy)]
struct Selection {
    anchor: usize,
    head: usize,
}

/// The state of a view between frames, in the temporary memory of egui.
#[derive(Clone)]
struct State {
    /// The hash of the text that the other fields describe.
    hash: u64,
    lines: Arc<Lines>,
    selection: Option<Selection>,
}

/// A text with what a frame needs to lay out its lines.
struct Text<'a> {
    code: &'a str,
    lines: &'a Lines,
    theme: &'a Theme,
    /// The top left of the first line.
    origin: egui::Pos2,
    row_height: f32,
}

impl<'a> SteelView<'a> {
    pub fn new(code: &'a str) -> Self {
        Self {
            code,
            highlights: &[],
            errors: &[],
            scroll_to: None,
        }
    }

    /// Byte ranges to emphasise with the selection colour.
    pub fn highlights(mut self, ranges: &'a [Range<usize>]) -> Self {
        self.highlights = ranges;
        self
    }

    /// Byte ranges to tint with the error colour.
    pub fn errors(mut self, ranges: &'a [Range<usize>]) -> Self {
        self.errors = ranges;
        self
    }

    /// Scroll the row containing this byte offset into view this frame.
    pub fn scroll_to(mut self, offset: Option<usize>) -> Self {
        self.scroll_to = offset;
        self
    }

    /// Render the view. Show it within a `ScrollArea`.
    pub fn show(self, ui: &mut egui::Ui) -> egui::Response {
        let id = ui.id().with("steel_view");
        let hash = egui::util::hash(self.code);
        let mut state = ui
            .data(|d| d.get_temp::<State>(id))
            .filter(|state| state.hash == hash)
            .unwrap_or_else(|| State {
                hash,
                lines: Arc::new(index(self.code)),
                selection: None,
            });

        let theme = Theme::of(ui);
        let (row_height, char_width) =
            ui.fonts_mut(|f| (f.row_height(&theme.font), f.glyph_width(&theme.font, '0')));
        // The view is at least as wide as the space at hand, for a press
        // beside a short text to reach it.
        let width = state.lines.max_chars as f32 * char_width;
        let available = Some(ui.available_width()).filter(|w| w.is_finite());
        let size = egui::vec2(
            available.map_or(width, |w| w.max(width)),
            line_count(&state.lines) as f32 * row_height,
        );
        let (_, rect) = ui.allocate_space(size);
        let response = ui
            .interact(rect, id, egui::Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::Text);
        let text = Text {
            code: self.code,
            lines: &state.lines,
            theme: &theme,
            origin: rect.min,
            row_height,
        };

        state.selection = select(ui, &text, &response, state.selection);
        let selected = state
            .selection
            .map(|s| s.anchor.min(s.head)..s.anchor.max(s.head))
            .filter(|range| !range.is_empty());
        if let Some(range) = selected.clone() {
            let copy = |e: &egui::Event| matches!(e, egui::Event::Copy | egui::Event::Cut);
            if ui.input(|i| i.events.iter().any(copy)) {
                ui.ctx().copy_text(self.code[range].to_string());
            }
        }

        if let Some(offset) = self.scroll_to {
            let target = cursor_rect(ui, &text, offset.min(self.code.len()));
            ui.scroll_to_rect(target, Some(egui::Align::Center));
        }

        let marks = [
            (
                self.highlights,
                ui.visuals().selection.bg_fill.gamma_multiply(0.4),
            ),
            (self.errors, ui.visuals().error_fg_color.gamma_multiply(0.3)),
        ];
        let selection_fill = ui.visuals().selection.bg_fill;
        let rows = visible_rows(
            rect.top(),
            ui.clip_rect().y_range(),
            row_height,
            line_count(&state.lines),
        );
        for row in rows {
            let line = line_range(self.code, &state.lines, row);
            let galley = line_galley(ui, &text, row);
            let top = rect.top() + row as f32 * row_height;
            let x = |byte: usize| rect.left() + x_at(&galley, &self.code[line.clone()], byte);
            let fill = |x: egui::Rangef, color: egui::Color32| {
                let span = egui::Rect::from_x_y_ranges(x, top..=top + row_height);
                ui.painter().rect_filled(span, 0.0, color);
            };
            for (ranges, color) in marks {
                for bytes in ranges.iter().filter_map(|range| clip(range, &line)) {
                    fill(egui::Rangef::new(x(bytes.start), x(bytes.end)), color);
                }
            }
            if let Some(sel) = &selected {
                // A selection that runs past the end of the line also covers
                // its newline. The newline shows as a short tail.
                let tail = match sel.start <= line.end && line.end < sel.end {
                    true => row_height / 2.0,
                    false => 0.0,
                };
                let bytes = clip(sel, &line).unwrap_or(line.len()..line.len());
                let span = egui::Rangef::new(x(bytes.start), x(bytes.end) + tail);
                if span.span() > 0.0 {
                    fill(span, selection_fill);
                }
            }
            ui.painter()
                .galley(egui::pos2(rect.left(), top), galley, theme.symbol);
        }

        ui.data_mut(|d| d.insert_temp(id, state));
        response
    }
}

/// The line starts and lexer modes of `code`. A text has one line more than
/// it has newlines. A text that ends in a newline ends in an empty line.
fn index(code: &str) -> Lines {
    let mut lines = Lines {
        starts: vec![0],
        modes: vec![],
        max_chars: 0,
    };
    let mut mode = Mode::Code;
    for line in code.split('\n') {
        let start = lines.starts[lines.starts.len() - 1];
        // The last line has no newline to step over.
        let end = (start + line.len() + 1).min(code.len());
        lines.modes.push(mode);
        lines.max_chars = lines.max_chars.max(line.chars().count());
        mode = syntax::mode_after(&code[start..end], mode);
        lines.starts.push(end);
    }
    lines
}

fn line_count(lines: &Lines) -> usize {
    lines.modes.len()
}

/// The byte range of line `row` of `code`, without its newline.
fn line_range(code: &str, lines: &Lines, row: usize) -> Range<usize> {
    let (start, end) = (lines.starts[row], lines.starts[row + 1]);
    start..start + code[start..end].trim_end_matches('\n').len()
}

/// The line of the byte at `offset`. An offset at the end of the text
/// is on the last line.
fn line_of(lines: &Lines, offset: usize) -> usize {
    let after = lines.starts[1..].partition_point(|&start| start <= offset);
    after.min(line_count(lines) - 1)
}

/// The rows that overlap the vertical range `clip`, of `count` rows of
/// `row_height` that start at `top`.
fn visible_rows(top: f32, clip: egui::Rangef, row_height: f32, count: usize) -> Range<usize> {
    let row = |y: f32, round: fn(f32) -> f32| {
        (round((y - top) / row_height).max(0.0) as usize).min(count)
    };
    let (first, last) = (row(clip.min, f32::floor), row(clip.max, f32::ceil));
    first..last.max(first)
}

/// The part of `range` within `line`, as offsets from the start of `line`.
/// `None` if they share no byte.
fn clip(range: &Range<usize>, line: &Range<usize>) -> Option<Range<usize>> {
    let (start, end) = (range.start.max(line.start), range.end.min(line.end));
    (start < end).then(|| start - line.start..end - line.start)
}

/// The range of the token around the byte at `offset`. It is empty at the
/// end of a line.
fn token_at(code: &str, lines: &Lines, offset: usize) -> Range<usize> {
    let row = line_of(lines, offset);
    let line = line_range(code, lines, row);
    syntax::tokens(&code[line.clone()], lines.modes[row])
        .map(|(_, range, _)| line.start + range.start..line.start + range.end)
        .find(|range| offset < range.end)
        .unwrap_or(line.end..line.end)
}

/// Line `row` of `text`, laid out in its syntax colours.
fn line_galley(ui: &egui::Ui, text: &Text, row: usize) -> Arc<egui::Galley> {
    let line = &text.code[line_range(text.code, text.lines, row)];
    let job = syntax::job(line, text.lines.modes[row], text.theme);
    ui.fonts_mut(|f| f.layout_job(job))
}

/// The distance from the left of the laid out `line` to its byte `byte`.
fn x_at(galley: &egui::Galley, line: &str, byte: usize) -> f32 {
    let chars = line[..byte].chars().count();
    galley
        .pos_from_cursor(egui::text::CCursor::new(chars))
        .min
        .x
}

/// The byte offset in `text` nearest to the screen position `pos`.
fn offset_at(ui: &egui::Ui, text: &Text, pos: egui::Pos2) -> usize {
    let last = line_count(text.lines) - 1;
    let row = (((pos.y - text.origin.y) / text.row_height).max(0.0) as usize).min(last);
    let line = line_range(text.code, text.lines, row);
    let galley = line_galley(ui, text, row);
    let cursor = galley.cursor_from_pos(egui::vec2(pos.x - text.origin.x, 0.0));
    let line_text = &text.code[line.clone()];
    let byte = line_text
        .char_indices()
        .nth(cursor.index)
        .map_or(line_text.len(), |(byte, _)| byte);
    line.start + byte
}

/// The screen rectangle of a text cursor before the byte at `offset`.
fn cursor_rect(ui: &egui::Ui, text: &Text, offset: usize) -> egui::Rect {
    let row = line_of(text.lines, offset);
    let line = line_range(text.code, text.lines, row);
    let byte = offset.clamp(line.start, line.end) - line.start;
    let x = x_at(&line_galley(ui, text, row), &text.code[line], byte);
    let min = text.origin + egui::vec2(x, row as f32 * text.row_height);
    egui::Rect::from_min_size(min, egui::vec2(0.0, text.row_height))
}

/// The selection after the input of this frame.
///
/// A press starts a selection and a drag moves its head. A press with shift
/// held moves the head of the selection at hand. A double click selects a
/// token and a triple click selects a line. Once the view has a selection,
/// the select-all shortcut selects the whole text. A press elsewhere or the
/// escape key drops the selection.
fn select(
    ui: &egui::Ui,
    text: &Text,
    response: &egui::Response,
    selection: Option<Selection>,
) -> Option<Selection> {
    let select_all = |e: &egui::Event| {
        let egui::Event::Key {
            key,
            pressed,
            modifiers,
            ..
        } = e
        else {
            return false;
        };
        *key == egui::Key::A && *pressed && modifiers.command
    };
    let (pressed, down, any_pressed, shift, escape, all) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.any_pressed(),
            i.modifiers.shift,
            i.key_pressed(egui::Key::Escape),
            i.events.iter().any(select_all),
        )
    });
    let head = response
        .interact_pointer_pos()
        .filter(|_| down && response.is_pointer_button_down_on())
        .map(|pos| offset_at(ui, text, pos));
    let selection = match (head, selection) {
        (Some(head), Some(sel)) if shift || !pressed => Some(Selection { head, ..sel }),
        (Some(head), _) => Some(Selection { anchor: head, head }),
        (None, _) if any_pressed || escape => None,
        (None, selection) => selection,
    };
    if let Some(head) = head.filter(|_| !pressed) {
        // Keep the head in view while the pointer drags past an edge.
        ui.scroll_to_rect(cursor_rect(ui, text, head), None);
    }
    let over = |range: Range<usize>| Selection {
        anchor: range.start,
        head: range.end,
    };
    match selection {
        Some(_) if all => Some(over(0..text.code.len())),
        Some(sel) if response.triple_clicked() => {
            let row = line_of(text.lines, sel.head);
            Some(over(text.lines.starts[row]..text.lines.starts[row + 1]))
        }
        Some(sel) if response.double_clicked() => {
            Some(over(token_at(text.code, text.lines, sel.head)))
        }
        selection => selection,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: egui::Vec2 = egui::vec2(400.0, 300.0);

    /// One frame of a view of `code` in a scroll area, with `events` as its
    /// input. Returns the output and the rectangle of the view.
    fn frame(
        ctx: &egui::Context,
        code: &str,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, egui::Rect) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)),
            events,
            ..Default::default()
        };
        let mut rect = egui::Rect::NOTHING;
        let output = ctx.run_ui(input, |ui| {
            egui::ScrollArea::both().show(ui, |ui| {
                rect = SteelView::new(code).show(ui).rect;
            });
        });
        (output, rect)
    }

    fn texts(output: &egui::FullOutput) -> Vec<String> {
        output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_string()),
                _ => None,
            })
            .collect()
    }

    fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// Press at `from`, drag to `to` and release.
    fn drag(ctx: &egui::Context, code: &str, from: egui::Pos2, to: egui::Pos2) {
        frame(ctx, code, vec![egui::Event::PointerMoved(from)]);
        frame(ctx, code, vec![button(from, true)]);
        frame(ctx, code, vec![egui::Event::PointerMoved(to)]);
        frame(ctx, code, vec![button(to, false)]);
    }

    /// The text that a copy event copies, if any.
    fn copy(ctx: &egui::Context, code: &str) -> Option<String> {
        let (output, _) = frame(ctx, code, vec![egui::Event::Copy]);
        output
            .platform_output
            .commands
            .iter()
            .find_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.clone()),
                _ => None,
            })
    }

    #[test]
    fn index_finds_line_starts_and_modes() {
        let lines = index("(a \"b\nc\")\n\nλλλλ");
        assert_eq!(lines.starts, vec![0, 6, 10, 11, 19]);
        assert_eq!(
            lines.modes,
            vec![Mode::Code, Mode::String, Mode::Code, Mode::Code]
        );
        assert_eq!(lines.max_chars, 5);
    }

    #[test]
    fn a_trailing_newline_ends_in_an_empty_line() {
        let code = "a\n";
        let lines = index(code);
        assert_eq!(lines.starts, vec![0, 2, 2]);
        assert_eq!(line_range(code, &lines, 0), 0..1);
        assert_eq!(line_range(code, &lines, 1), 2..2);
        assert_eq!(index("").starts, vec![0, 0]);
    }

    #[test]
    fn line_of_maps_offsets_to_lines() {
        let lines = index("ab\n\ncd");
        let rows: Vec<_> = (0..=6).map(|offset| line_of(&lines, offset)).collect();
        assert_eq!(rows, vec![0, 0, 0, 1, 2, 2, 2]);
    }

    #[test]
    fn visible_rows_cover_the_clip() {
        let clip = |min, max| egui::Rangef::new(min, max);
        assert_eq!(visible_rows(0.0, clip(0.0, 100.0), 10.0, 1000), 0..10);
        assert_eq!(visible_rows(-55.0, clip(0.0, 100.0), 10.0, 1000), 5..16);
        assert_eq!(visible_rows(0.0, clip(0.0, 100.0), 10.0, 3), 0..3);
        assert_eq!(visible_rows(500.0, clip(0.0, 100.0), 10.0, 3), 0..0);
        assert_eq!(visible_rows(-500.0, clip(0.0, 100.0), 10.0, 3), 3..3);
    }

    #[test]
    fn clip_is_relative_to_the_line() {
        assert_eq!(clip(&(3..8), &(5..12)), Some(0..3));
        assert_eq!(clip(&(6..8), &(5..12)), Some(1..3));
        assert_eq!(clip(&(0..5), &(5..12)), None);
        assert_eq!(clip(&(7..7), &(5..12)), None);
    }

    #[test]
    fn token_at_finds_the_token_under_an_offset() {
        let code = "(define x\n  \"a b\")";
        let lines = index(code);
        assert_eq!(&code[token_at(code, &lines, 3)], "define");
        assert_eq!(&code[token_at(code, &lines, 14)], "\"a b\"");
        assert_eq!(token_at(code, &lines, code.len()), code.len()..code.len());
    }

    /// Only the lines in view reach the painter, however long the text.
    #[test]
    fn lays_out_only_the_lines_in_view() {
        let code: String = (0..10_000)
            .map(|i| format!("(define x{i} {i})\n"))
            .collect();
        let ctx = egui::Context::default();
        let (output, rect) = frame(&ctx, &code, vec![]);
        let texts = texts(&output);
        assert_eq!(texts[0], "(define x0 0)");
        let row_height = rect.height() / 10_001.0;
        assert!((10.0..30.0).contains(&row_height), "{row_height}");
        let in_view = (SIZE.y / row_height).ceil() as usize;
        assert!(texts.len() <= in_view + 1, "{} lines", texts.len());
    }

    /// A drag selects the text between the press and the release, and a copy
    /// event copies it.
    #[test]
    fn drag_selects_and_copy_event_copies() {
        let code = "(define x 1)\n(define y 2)\n(define z 3)\n";
        let ctx = egui::Context::default();
        let (_, rect) = frame(&ctx, code, vec![]);
        let row_height = rect.height() / 4.0;
        let from = rect.min + egui::vec2(0.0, 0.5 * row_height);
        let to = egui::pos2(rect.right() - 1.0, rect.top() + 1.5 * row_height);
        drag(&ctx, code, from, to);
        assert_eq!(
            copy(&ctx, code).as_deref(),
            Some("(define x 1)\n(define y 2)")
        );

        // A press outside the view drops the selection.
        let outside = egui::pos2(SIZE.x - 30.0, SIZE.y - 30.0);
        assert!(!rect.contains(outside));
        drag(&ctx, code, outside, outside);
        assert_eq!(copy(&ctx, code), None);
    }

    /// A selection can start on a line that has scrolled out of view.
    #[test]
    fn a_drag_past_the_bottom_selects_lines_out_of_view() {
        let code: String = (0..200).map(|i| format!("(line {i})\n")).collect();
        let ctx = egui::Context::default();
        let (_, rect) = frame(&ctx, &code, vec![]);
        let row_height = rect.height() / 201.0;
        let from = rect.min + egui::vec2(0.0, 0.5 * row_height);
        // The pointer leaves the window by its bottom edge while it drags.
        let to = egui::pos2(rect.left(), SIZE.y + 20.0 * row_height);
        drag(&ctx, &code, from, to);
        let copied = copy(&ctx, &code).expect("a selection");
        let rows = copied.lines().count();
        assert!(copied.starts_with("(line 0)\n(line 1)\n"));
        assert!(rows as f32 > SIZE.y / row_height, "{rows} lines");
    }

    /// After a click in the view, the select-all shortcut selects the whole
    /// text, lines out of view included.
    #[test]
    fn select_all_selects_the_whole_text() {
        let code: String = (0..200).map(|i| format!("(line {i})\n")).collect();
        let ctx = egui::Context::default();
        let (_, rect) = frame(&ctx, &code, vec![]);
        let select_all = egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        frame(&ctx, &code, vec![select_all.clone()]);
        assert_eq!(copy(&ctx, &code), None);
        let at = rect.min + egui::vec2(2.0, 2.0);
        drag(&ctx, &code, at, at);
        frame(&ctx, &code, vec![select_all]);
        assert_eq!(copy(&ctx, &code).as_deref(), Some(code.as_str()));
    }

    /// The offsets of a selection belong to the text it was made in. A new
    /// text drops it.
    #[test]
    fn a_new_text_drops_the_selection() {
        let code = "(define x 1)\n(define y 2)";
        let ctx = egui::Context::default();
        let (_, rect) = frame(&ctx, code, vec![]);
        drag(
            &ctx,
            code,
            rect.min + egui::vec2(0.0, 2.0),
            rect.max - egui::vec2(1.0, 2.0),
        );
        assert!(copy(&ctx, code).is_some());
        assert_eq!(copy(&ctx, "(define x 2)"), None);
    }
}
