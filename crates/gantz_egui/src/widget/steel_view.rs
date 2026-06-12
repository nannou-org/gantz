//! A read-only view of compiled steel source with byte-range highlighting
//! (e.g. the selected node's emitted fns and call sites, diagnostic spans)
//! and scroll-to support.

use egui::text::{LayoutJob, LayoutSection};
use std::ops::Range;

/// A syntax-highlighted, read-only steel source view.
pub struct SteelView<'a> {
    code: &'a str,
    /// Byte ranges to emphasise (e.g. the selected node's spans).
    highlights: &'a [Range<usize>],
    /// Byte ranges of diagnostic spans, tinted with the error colour.
    errors: &'a [Range<usize>],
    /// A byte offset to scroll into view this frame.
    scroll_to: Option<usize>,
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

    /// Render the view (expects to be shown within a `ScrollArea`).
    pub fn show(self, ui: &mut egui::Ui) -> egui::Response {
        let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(ui.ctx(), ui.style());
        let mut job = egui_extras::syntax_highlighting::highlight(
            ui.ctx(),
            ui.style(),
            &theme,
            self.code,
            "scm",
        );
        let highlight_bg = ui.visuals().selection.bg_fill.gamma_multiply(0.4);
        let error_bg = ui.visuals().error_fg_color.gamma_multiply(0.3);
        apply_background(&mut job, self.highlights, highlight_bg);
        apply_background(&mut job, self.errors, error_bg);
        job.wrap.max_width = f32::INFINITY;

        let galley = ui.painter().layout_job(job);
        let response = ui.add(egui::Label::new(galley.clone()).selectable(true));

        if let Some(offset) = self.scroll_to {
            let chars = self.code[..offset.min(self.code.len())].chars().count();
            let row = galley.pos_from_cursor(egui::text::CCursor::new(chars));
            let target = row.translate(response.rect.min.to_vec2());
            ui.scroll_to_rect(target, Some(egui::Align::Center));
        }
        response
    }
}

/// Set the background colour of the given byte ranges, splitting the job's
/// sections at range boundaries as needed.
pub(crate) fn apply_background(job: &mut LayoutJob, ranges: &[Range<usize>], color: egui::Color32) {
    for range in ranges {
        if range.start >= range.end {
            continue;
        }
        let mut sections = Vec::with_capacity(job.sections.len() + 2);
        for section in job.sections.drain(..) {
            let sr = section.byte_range.clone();
            if sr.end <= range.start || range.end <= sr.start {
                sections.push(section);
                continue;
            }
            let mid = sr.start.max(range.start)..sr.end.min(range.end);
            let mut first = true;
            for (bytes, within) in [
                (sr.start..mid.start, false),
                (mid.clone(), true),
                (mid.end..sr.end, false),
            ] {
                if bytes.start >= bytes.end {
                    continue;
                }
                let mut format = section.format.clone();
                if within {
                    format.background = color;
                }
                sections.push(LayoutSection {
                    // Leading space belongs to the first sub-section only.
                    leading_space: if first { section.leading_space } else { 0.0 },
                    byte_range: bytes,
                    format,
                });
                first = false;
            }
        }
        job.sections = sections;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Color32;

    fn job() -> LayoutJob {
        let mut job = LayoutJob::default();
        job.append("hello", 0.0, Default::default());
        job.append(" world!", 0.0, Default::default());
        job
    }

    fn ranges(job: &LayoutJob) -> Vec<(Range<usize>, bool)> {
        job.sections
            .iter()
            .map(|s| {
                (
                    s.byte_range.clone(),
                    s.format.background != Color32::default(),
                )
            })
            .collect()
    }

    #[test]
    fn splits_sections_at_range_boundaries() {
        let mut job = job();
        apply_background(&mut job, &[3..8], Color32::YELLOW);
        assert_eq!(
            ranges(&job),
            vec![(0..3, false), (3..5, true), (5..8, true), (8..12, false)]
        );
        // Full text preserved in order.
        assert_eq!(job.sections.last().unwrap().byte_range.end, job.text.len());
    }

    #[test]
    fn empty_and_out_of_range_are_noops() {
        let mut job = job();
        apply_background(&mut job, &[4..4, 50..60], Color32::YELLOW);
        assert_eq!(ranges(&job), vec![(0..5, false), (5..12, false)]);
    }

    #[test]
    fn overlapping_ranges_compose() {
        let mut job = job();
        apply_background(&mut job, &[0..4, 2..6], Color32::YELLOW);
        let r = ranges(&job);
        // Every byte in 0..6 highlighted, the rest untouched.
        assert!(r.iter().all(|(range, bg)| *bg == (range.start < 6)));
        assert_eq!(r.last().unwrap().0.end, 12);
    }
}
