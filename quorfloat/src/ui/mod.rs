//! What the panel draws, and nothing else.
//!
//! Separate from [`crate::app`] along one line: this module turns a `PanelState` into
//! pixels and reports clicks as `CardAction`s, while `app` owns the state, the
//! lifecycle, and the protocol. The separation is what makes "the panel showed the wrong
//! thing" and "the panel knew the wrong thing" different questions — the first is here,
//! the second is not.
//!
//! Nothing here can reach the session: the whole input is the state it is handed, and the
//! whole output is the action it returns. That is also why the tests can draw a panel in
//! a headless egui context and assert where the text landed.

mod cards;
mod conversation;
pub mod fonts;
mod theme;
pub mod window;

pub use cards::CardAction;
/// The speaker label and the requested locale are asserted by tests in `app`, so they are
/// reachable from there — and from nowhere else in a normal build.
#[cfg(test)]
pub(crate) use conversation::speaker;
#[cfg(test)]
pub(crate) use theme::DISPLAY_LOCALE;
use cards::{approval_card, handoff_banner, question_card};
use conversation::conversation;
use theme::{BACKGROUND, MUTED, TEXT, WARN};

use eframe::egui;

use crate::app::PanelState;
use crate::app::session::interaction::InteractionKind;

/// Draw the panel.
///
/// Clicks are reported rather than applied: the caller owns the session, and holding its
/// lock inside a layout closure is how a repaint becomes a deadlock.
///
/// @param ui - the root area, with no margin or background of its own.
/// @param state - everything the panel is allowed to know.
/// @param action - where a click is reported, if the user makes one.
pub(crate) fn draw(ui: &mut egui::Ui, state: &PanelState, action: &mut Option<CardAction>) {
    let hotkey_status = state.hotkey.clone();
    let follow_status = state.follow.status();
    let fonts_warning = state.fonts_warning.clone();

    egui::Frame::NONE
        .fill(BACKGROUND)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            // The harness's own title when it has one: it is the name the user sees in
            // the Harness window, so the panel says the same thing rather than guessing
            // from the first message.
            let heading = state.title.clone().unwrap_or_else(|| "quorfloat".to_owned());
            wrapped(ui, egui::RichText::new(heading).size(15.0).color(TEXT));
            ui.add_space(2.0);
            ui.label(egui::RichText::new(hotkey_status).size(12.0).color(MUTED));
            ui.add_space(2.0);
            ui.label(egui::RichText::new(follow_status).size(12.0).color(MUTED));
            if let Some(warning) = &fonts_warning {
                ui.add_space(2.0);
                wrapped(ui, egui::RichText::new(warning).size(12.0).color(WARN));
            }
            ui.add_space(8.0);

            if let Some(handoff) = &state.handoff {
                handoff_banner(ui, handoff, action);
                ui.add_space(8.0);
            }

            if state.interactions.is_empty() {
                ui.label(egui::RichText::new("没有待处理的请求").size(12.0).color(MUTED));
            } else {
                // `auto_shrink` vertically, and bounded: a request is actionable so it has
                // to be on screen, but a year-old card must not push the conversation out
                // of the panel — which is exactly what an `auto_shrink([false, false])`
                // area does even when it is empty.
                let cards = (ui.available_height() * 0.6).max(120.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .max_height(cards)
                    .show(ui, |ui| {
                        for card in &state.interactions {
                            match card.kind() {
                                InteractionKind::Approval => approval_card(ui, card, action),
                                InteractionKind::Question => question_card(ui, card, action),
                            }
                            ui.add_space(8.0);
                        }
                    });
            }

            ui.add_space(8.0);
            conversation(ui, state);
        });
}

/// A label that wraps instead of being clipped.
///
/// egui's default is to truncate a long line, which for a tool result means the user
/// cannot read what the tool said — and the whole point of showing it is that they can.
///
/// @param ui - where to draw.
/// @param text - the text.
pub(super) fn wrapped(ui: &mut egui::Ui, text: egui::RichText) {
    ui.add(egui::Label::new(text).wrap());
}
