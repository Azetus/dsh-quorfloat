//! The conversation: the lines, and who they are from.

use eframe::egui;

use crate::app::PanelState;
use crate::ui::theme::{BAD, CONVERSATION_MIN_HEIGHT, MUTED, TEXT};

use super::wrapped;

/// Draw the conversation.
///
/// Sticks to the bottom, which is the whole of the "follow the answer as it is written"
/// behaviour: egui only keeps a scroll area pinned while it is already at the bottom, so
/// a user who has scrolled up to read something is not yanked back by the next token.
///
/// @param ui - where to draw.
/// @param state - the conversation, plus the message still being generated.
pub(super) fn conversation(ui: &mut egui::Ui, state: &PanelState) {
    // Whatever the header and any cards did not take. This is the panel's main body, so
    // it is the part that must never be squeezed to nothing: an empty area that expands
    // to fill the window is what left the conversation invisible, not empty.
    let height = ui.available_height().max(CONVERSATION_MIN_HEIGHT);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .max_height(height)
        .show(ui, |ui| {
            if state.entries.is_empty() && state.live.is_none() {
                ui.label(egui::RichText::new("尚无对话内容").size(12.0).color(MUTED));
                return;
            }
            for entry in state.entries.iter() {
                entry_ui(ui, entry);
            }
            if let Some(live) = &state.live {
                entry_ui(ui, live);
            }
        });
}

/// Draw one line of the conversation.
///
/// @param ui - where to draw.
/// @param entry - the line.
pub(super) fn entry_ui(ui: &mut egui::Ui, entry: &crate::app::session::transcript::Entry) {
    use crate::app::session::transcript::{Block, Entry};

    ui.add_space(6.0);
    ui.label(egui::RichText::new(speaker(entry)).size(11.0).color(MUTED));
    ui.add_space(2.0);
    match entry {
        Entry::User { text } => {
            wrapped(ui, egui::RichText::new(text).size(13.0).color(TEXT));
        }
        Entry::Assistant { blocks, streaming } => {
            for block in blocks {
                match block {
                    Block::Text(text) => wrapped(ui, egui::RichText::new(text).size(13.0).color(TEXT)),
                    // Reasoning is drawn, not hidden: it is what the model is doing, and
                    // a panel that shows only conclusions makes a slow answer look stuck.
                    Block::Reasoning(text) => {
                        wrapped(ui, egui::RichText::new(text).size(12.0).color(MUTED).italics());
                    }
                    Block::Call { name, arguments } => {
                        wrapped(ui, egui::RichText::new(format!("$ {name} {arguments}")).size(12.0).color(MUTED).monospace());
                    }
                }
                ui.add_space(2.0);
            }
            if *streaming {
                ui.label(egui::RichText::new("生成中…").size(11.0).color(MUTED));
            }
        }
        Entry::Tool { text, is_error, .. } => {
            let colour = if *is_error { BAD } else { MUTED };
            wrapped(ui, egui::RichText::new(text).size(12.0).color(colour).monospace());
        }
        Entry::System { text } => wrapped(ui, egui::RichText::new(text).size(12.0).color(MUTED)),
        Entry::Notice { text, .. } => wrapped(ui, egui::RichText::new(text).size(12.0).color(MUTED)),
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
