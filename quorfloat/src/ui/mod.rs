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
mod composer;
mod conversation;
mod geometry;
pub mod fonts;
mod theme;
pub mod window;

/// The speaker label and the requested locale are asserted by tests in `app`, so they are
/// reachable from there — and from nowhere else in a normal build.
#[cfg(test)]
pub(crate) use conversation::speaker;
#[cfg(test)]
pub(crate) use theme::DISPLAY_LOCALE;
use cards::{approval_card, handoff_banner, question_card};
use composer::composer;
use conversation::conversation;
use theme::{BACKGROUND, MUTED, TEXT, WARN};

use eframe::egui;

use crate::app::PanelState;
use crate::app::session::interaction::{ApprovalVerdict, InteractionKind};

pub use geometry::WindowState;

/// What the user asked for, in one place.
///
/// The whole output of this layer: it draws a state and reports intent, and `app` decides
/// what intent is allowed. An approval and a prompt are the same kind of value here — both
/// are things the user did, not things the panel does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Answer one approval.
    Answer {
        /// Which interaction.
        id: String,
        /// What the user decided.
        verdict: ApprovalVerdict,
    },
    /// Stop showing a card.
    Dismiss {
        /// Which interaction.
        id: String,
    },
    /// Stop showing the hand-off banner.
    DismissHandoff,
    /// Send what the user typed.
    Send {
        /// The text, as typed.
        text: String,
    },
    /// Ask the host to stop the turn in flight.
    Cancel,
}

/// Draw the panel.
///
/// Clicks are reported rather than applied: the caller owns the session, and holding its
/// lock inside a layout closure is how a repaint becomes a deadlock.
///
/// @param ui - the root area, with no margin or background of its own.
/// @param state - everything the panel is allowed to know.
/// @param draft - the composer's text, owned by the caller so it survives a frame.
/// @param action - where a click is reported, if the user makes one.
pub(crate) fn draw(ui: &mut egui::Ui, state: &PanelState, draft: &mut String, action: &mut Option<Action>) {
    let hotkey_status = state.hotkey.clone();
    let follow_status = state.follow.status();
    let fonts_warning = state.fonts_warning.clone();

    egui::Frame::NONE
        .fill(BACKGROUND)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            // The header doubles as the panel's drag handle: an undecorated window has no
            // title bar, so without this the panel cannot be moved at all. The whole header
            // rather than just the title, because the target should be as large as the
            // thing looks — but the status lines stay *selectable*, and a drag that starts
            // on selectable text selects it, so copying a session id still works.
            let header = ui
                .scope(|ui| {
                    // The title, though, is not text to select: it is the part of the panel
                    // that behaves like a handle, and a selection there would eat the drag.
                    ui.style_mut().interaction.selectable_labels = false;
                    let heading = state.title.clone().unwrap_or_else(|| "quorfloat".to_owned());
                    wrapped(ui, egui::RichText::new(heading).size(15.0).color(TEXT));
                    // Back on for everything below it.
                    ui.style_mut().interaction.selectable_labels = true;
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new(hotkey_status).size(12.0).color(MUTED));
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new(follow_status).size(12.0).color(MUTED));
                    if let Some(warning) = &fonts_warning {
                        ui.add_space(2.0);
                        wrapped(ui, egui::RichText::new(warning).size(12.0).color(WARN));
                    }
                })
                .response;
            let handle = ui.interact(header.rect, ui.id().with("quorfloat-drag"), egui::Sense::drag());
            if handle.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if handle.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
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
            // The composer's strip is reserved *before* the conversation is laid out.
            // Order is the fix: drawn last with no reservation, the composer paid for a
            // conversation that grew, and a long conversation pushed the input box — the
            // one control that must always be reachable — past the bottom of the window.
            let strip = composer::height(ui, state);
            conversation(ui, state, ui.available_height() - strip);
            ui.add_space(8.0);
            composer(ui, state, draft, action);
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
