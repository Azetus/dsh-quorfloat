//! Where a table stops being the viewer's problem.
//!
//! The Markdown viewer draws a table as an `egui::Grid`, which measures its columns from the
//! content and has no horizontal scroll of its own — so one wide cell asks for more width than the
//! panel has and paints past its edge.
//!
//! The answer is to bound the table and nothing else: [`draw`] renders an answer through the viewer
//! exactly as before, except that each table is rendered inside a horizontal scroll area the width
//! of the panel, which is a boundary the viewer cannot cross whatever it draws. Prose, lists and
//! code are untouched, and the library keeps owning how Markdown becomes widgets.
//!
//! The cutting is the parser's too. `pulldown-cmark` is the parser `egui_commonmark` itself uses,
//! and it already reports the byte range of every table (`into_offset_iter`), so nothing here
//! re-implements Markdown: a fence containing a `|`, an escaped `\|`, a table indented inside a
//! list item — all of it is decided where CommonMark says it is decided.

use std::ops::Range;

use eframe::egui;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// The byte ranges of the tables in a Markdown document.
///
/// @param text - the document.
/// @returns one range per table, in document order.
#[must_use]
pub(crate) fn table_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut opened: Option<usize> = None;
    for (event, range) in Parser::new_ext(text, Options::all()).into_offset_iter() {
        match event {
            Event::Start(Tag::Table(_)) => opened = Some(range.start),
            Event::End(TagEnd::Table) => {
                if let Some(start) = opened.take() {
                    ranges.push(start..range.end);
                }
            }
            _ => {}
        }
    }
    ranges
}

/// Draw an answer, with every table bounded by the panel's width.
///
/// @param ui - where to draw.
/// @param markdown - the viewer's cache.
/// @param text - the answer's Markdown source.
/// @param available - the width there is to draw in.
pub(crate) fn draw(
    ui: &mut egui::Ui,
    markdown: &mut egui_commonmark::CommonMarkCache,
    text: &str,
    available: f32,
) {
    let mut cursor = 0;
    for range in table_ranges(text) {
        if range.start > cursor {
            prose(ui, markdown, &text[cursor..range.start], available);
        }
        // A table scrolls sideways rather than pushing the panel open. The scroll area exists for
        // this one case and is inert when the table happens to fit.
        egui::ScrollArea::horizontal()
            .max_width(available)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                viewer(ui, markdown, &text[range.clone()], available);
            });
        cursor = range.end;
    }
    if cursor < text.len() {
        prose(ui, markdown, &text[cursor..], available);
    }
}

/// Draw a stretch of prose, giving the layout somewhere to break inside long runs.
///
/// egui wraps at word boundaries and never inside a word unless it is truncating, so a code block
/// holding one long JSON line would be laid out at its full width. A zero-width space inside long
/// runs gives the wrapper a candidate; it draws nothing, and the text a copy takes is the untouched
/// source (`conversation::answer_source`), so nothing machine-readable changes.
fn prose(
    ui: &mut egui::Ui,
    markdown: &mut egui_commonmark::CommonMarkCache,
    slice: &str,
    available: f32,
) {
    if slice.trim().is_empty() {
        return;
    }
    let wrapped = super::conversation::soft_wrap_for_display(slice);
    viewer(ui, markdown, &wrapped, available);
}

/// Hand one slice to the viewer.
fn viewer(
    ui: &mut egui::Ui,
    markdown: &mut egui_commonmark::CommonMarkCache,
    slice: &str,
    available: f32,
) {
    egui_commonmark::CommonMarkViewer::new()
        .default_width(Some(available as usize))
        .show(ui, markdown, slice);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_is_cut_out_of_the_prose_around_it() {
        let answer = "先看这张表：\n\n| 方案 | 做法 |\n| --- | --- |\n| A | 滚动 |\n\n然后继续说明。\n";
        let ranges = table_ranges(answer);
        assert_eq!(ranges.len(), 1, "{ranges:?}");
        let table = &answer[ranges[0].clone()];
        assert!(table.contains("| 方案 | 做法 |"), "{table:?}");
        assert!(table.contains("| A | 滚动 |"), "{table:?}");
        assert!(!table.contains("然后继续说明"), "the prose is not part of it: {table:?}");
    }

    /// The reason this is the parser's job and not a line scanner's: CommonMark decides.
    #[test]
    fn a_pipe_inside_a_code_fence_is_not_a_table() {
        let answer = "例子：\n\n```sh\n| a | b |\n| - | - |\n```\n\n结束\n";
        assert!(table_ranges(answer).is_empty(), "{:?}", table_ranges(answer));
    }

    #[test]
    fn an_escaped_pipe_is_not_a_cell_boundary() {
        let answer = "| a | b |\n| - | - |\n| `x \\| y` | 2 |\n";
        assert_eq!(table_ranges(answer).len(), 1);
    }

    #[test]
    fn a_table_inside_a_list_item_is_still_found() {
        let answer = "- 第一步：\n\n  | a | b |\n  | - | - |\n  | 1 | 2 |\n\n- 第二步\n";
        assert_eq!(table_ranges(answer).len(), 1, "{:?}", table_ranges(answer));
    }

    #[test]
    fn ranges_are_ordered_and_do_not_overlap() {
        let answer = "一\n\n| a |\n| - |\n| 1 |\n\n二\n\n| b |\n| - |\n| 2 |\n\n三\n";
        let ranges = table_ranges(answer);
        assert_eq!(ranges.len(), 2, "{ranges:?}");
        assert!(ranges[0].end <= ranges[1].start, "{ranges:?}");
        for range in &ranges {
            assert!(range.start < range.end && range.end <= answer.len(), "{range:?}");
        }
        // And every byte is either in a table or in the prose between them.
        let covered: usize = ranges.iter().map(|range| range.end - range.start).sum();
        assert!(covered < answer.len(), "the prose between them is not claimed");
    }
}
