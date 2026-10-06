//! The conversation: the lines, and who they are from.

use eframe::egui;

use crate::app::PanelState;
use crate::ui::theme as theme;
use crate::ui::theme::CONVERSATION_MIN_HEIGHT;

use super::wrapped;

/// Draw the conversation.
///
/// Sticks to the bottom, which is the whole of the "follow the answer as it is written"
/// behaviour: egui only keeps a scroll area pinned while it is already at the bottom, so
/// a user who has scrolled up to read something is not yanked back by the next token.
///
/// @param ui - where to draw.
/// @param state - the conversation, plus the message still being generated.
///
/// @param height - the space left after the header, the cards and the composer's strip.
///   Passed in rather than measured so that the composer, which is drawn afterwards, has
///   already claimed its share.
///
/// @returns how tall the conversation's content is, which is what the window's height is
///   grown from. The height handed in is the space it may *use*; this is the space it
///   *wants*, and the two differ exactly when the conversation is short.
pub(super) fn conversation(ui: &mut egui::Ui, state: &PanelState, height: f32) -> f32 {
    // The floor is a floor for a *window*, not a claim on space that is not there: past it
    // the conversation is clipped at the bottom — above the composer, which is the right
    // thing to lose.
    let height = height.max(CONVERSATION_MIN_HEIGHT.min(height.max(0.0))).max(0.0);
    let output = egui::ScrollArea::vertical()
        // Shrink to the content vertically, fill horizontally. This is what keeps the drawn
        // panel the same height as the height the window was asked for: with
        // `auto_shrink([false, false])` the area *fills* whatever it is offered, so a short
        // conversation drew 57px of empty space where the layout had counted 17 — and the
        // panel came out 40px taller than its window, with the footer clipped off the bottom
        // (see `docs/prototype.md` §30).
        .auto_shrink([false, true])
        .stick_to_bottom(true)
        .max_height(height)
        .show(ui, |ui| {
            if state.entries.is_empty() && state.live.is_none() {
                ui.label(theme::meta(ui.ctx(), "尚无对话内容"));
                return;
            }
            for entry in state.entries.iter() {
                entry_ui(ui, entry);
            }
            if let Some(live) = &state.live {
                entry_ui(ui, live);
            }
        });
    output.content_size.y
}

/// A model's answer, in the design's own proportion.
///
/// The line height is the point: an answer is prose to be read rather than scanned, and
/// `line-height:1.85` is what the design uses to keep a long one readable. It is set on the
/// text rather than through the style so that a wrapped paragraph and the reasoning block
/// beside it do not drift apart.
///
/// @param ctx - the context, for the font the weights are cut from.
/// @param text - the answer so far.
/// @returns the text to lay out.
fn answer(ctx: &egui::Context, text: &str) -> egui::RichText {
    egui::RichText::new(text)
        .font(theme::font(ctx, theme::Weight::Regular, theme::TEXT_BODY))
        .line_height(Some(theme::LINE_ANSWER))
        .color(theme::text())
}

/// Draw one line of the conversation.
///
/// @param ui - where to draw.
/// @param entry - the line.
pub(super) fn entry_ui(ui: &mut egui::Ui, entry: &crate::app::session::transcript::Entry) {
    use crate::app::session::transcript::{Block, Entry};

    ui.add_space(6.0);
    ui.label(egui::RichText::new(speaker(entry)).size(theme::TEXT_META).color(theme::muted()));
    ui.add_space(2.0);
    match entry {
        Entry::User { text } => {
            wrapped(ui, answer(ui.ctx(), text));
        }
        Entry::Assistant { blocks, streaming } => {
            for block in blocks {
                match block {
                    Block::Text(text) => wrapped(ui, answer(ui.ctx(), text)),
                    // Reasoning is drawn, not hidden: it is what the model is doing, and
                    // a panel that shows only conclusions makes a slow answer look stuck.
                    Block::Reasoning(text) => {
                        wrapped(ui, egui::RichText::new(text).size(theme::TEXT_META).color(theme::muted()).italics());
                    }
                    Block::Call { name, arguments } => {
                        wrapped(ui, egui::RichText::new(format!("$ {name} {arguments}")).size(theme::TEXT_META).color(theme::muted()).monospace());
                    }
                }
                ui.add_space(2.0);
            }
            if *streaming {
                ui.label(egui::RichText::new("生成中…").size(theme::TEXT_SMALL).color(theme::muted()));
            }
        }
        Entry::Tool { text, is_error, .. } => {
            let colour = if *is_error { theme::bad_text() } else { theme::muted() };
            wrapped(ui, egui::RichText::new(text).size(theme::TEXT_META).color(colour).monospace());
        }
        Entry::System { text } => wrapped(ui, theme::meta(ui.ctx(), text)),
        Entry::Notice { text, .. } => wrapped(ui, theme::meta(ui.ctx(), text)),
    }
}

/// Who a line is from, as the panel labels it.
///
/// @param entry - the line.
/// @returns the label above it.
#[must_use]
pub(crate) fn speaker(entry: &crate::app::session::transcript::Entry) -> String {
    use crate::app::session::transcript::Entry;
    match entry {
        Entry::User { .. } => "你".to_owned(),
        Entry::Assistant { .. } => "quorfloat".to_owned(),
        Entry::Tool { name, .. } => {
            name.as_ref().map_or_else(|| "工具".to_owned(), |name| format!("工具 · {name}"))
        }
        Entry::System { .. } => "系统".to_owned(),
        // The kind is in the label rather than only in the text: an unrecognised event
        // is exactly the case where the reader needs to know what it was called.
        Entry::Notice { kind, .. } => format!("事件 · {kind}"),
    }
}
