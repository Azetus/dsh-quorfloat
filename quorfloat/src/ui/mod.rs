//! What the panel draws, and nothing else.
//!
//! Separate from [`crate::app`] along one line: this module turns a `PanelState` into
//! pixels and reports clicks as `Action`s, while `app` owns the state, the lifecycle, and
//! the protocol. The separation is what makes "the panel showed the wrong thing" and "the
//! panel knew the wrong thing" different questions — the first is here, the second is not.
//!
//! Nothing here can reach the session: the whole input is the state it is handed, and the
//! whole output is the action it returns. That is also why the tests can draw a panel in
//! a headless egui context and assert where the text landed.
//!
//! **The order of the sections is the design's**, top to bottom: the bar that says what
//! this panel is attached to, the composer, whatever needs an answer, the conversation,
//! and the footer. The composer sits *above* the conversation rather than below it, which
//! is the one structural claim the design makes: this is a thing you type into, and the
//! answer grows underneath as you read it.

mod cards;
mod composer;
mod conversation;
mod geometry;
pub mod fonts;
pub mod icons;
pub mod theme;
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
use theme::{bg, line, muted, text, warn_text};

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
    /// Put the panel away without losing the conversation.
    Hide,
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
    // The window is transparent so that the panel can have rounded corners and a shadow of
    // its own; this is the room it leaves for both.
    egui::Frame::NONE
        .inner_margin(egui::Margin {
            left: theme::SHADOW_ROOM_SIDE,
            right: theme::SHADOW_ROOM_SIDE,
            top: theme::SHADOW_ROOM_TOP,
            bottom: theme::SHADOW_ROOM_BOTTOM,
        })
        .show(ui, |ui| {
            // The far shadow is painted before the panel and the near one by the panel's own
            // frame: the design stacks two layers, and one `Frame` carries one.
            let rect = ui.available_rect_before_wrap();
            ui.painter().add(theme::shadow_far().as_shape(
                rect,
                egui::CornerRadius::same(theme::RADIUS_WINDOW),
            ));
            egui::Frame::NONE
                .fill(bg())
                .corner_radius(egui::CornerRadius::same(theme::RADIUS_WINDOW))
                .stroke(egui::Stroke::new(theme::BORDER, line()))
                .shadow(theme::shadow_near())
                .show(ui, |ui| {
                    top_bar(ui, state, action);
                    composer(ui, state, draft, action);
                    if let Some(handoff) = &state.handoff {
                        handoff_banner(ui, handoff, action);
                    }
                    cards(ui, state, action);
                    // The composer claimed its share by being drawn first; the footer is
                    // below the conversation and has to be predicted, or a long
                    // conversation pushes the panel's own hints off the bottom.
                    let footer = footer_height(ui);
                    conversation(ui, state, ui.available_height() - footer);
                    footer_bar(ui, state);
                });
        });
}

/// The eight pixels of nothing that keep two sections from touching.
const SECTION_GAP: f32 = 8.0;

/// The bar at the top: what this panel is attached to, and the way out.
///
/// It is also the panel's drag handle. An undecorated window has no title bar, so without
/// it the panel cannot be moved at all — and it is the whole bar rather than just the name,
/// because the target should be as large as the thing looks. The text inside stays
/// *selectable* apart from the session name, so a drag that starts on the metadata selects
/// it and a drag that starts on the name moves the window.
fn top_bar(ui: &mut egui::Ui, state: &PanelState, action: &mut Option<Action>) {
    let bar = ui
        .scope(|ui| {
            ui.style_mut().interaction.selectable_labels = false;
            let frame = egui::Frame::NONE.inner_margin(theme::PAD_TOP);
            frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = theme::GAP_TIGHT;
                    // The brand, at the design's weight and letter spacing: it is a mark, not
                    // a sentence, and it never changes.
                    ui.label(
                        egui::RichText::new("QUORFLOAT")
                            .font(theme::font(ui.ctx(), theme::Weight::Medium, theme::TEXT_BRAND))
                            .extra_letter_spacing(theme::BRAND_LETTER_SPACING)
                            .color(text()),
                    );
                    ui.add_space(theme::GAP);
                    // What the panel is looking at. A picker in the design; here it is the
                    // name alone until the conversation list exists to put behind it — a
                    // caret that opens nothing is a promise the panel cannot keep.
                    ui.label(icons::glyph(ui.ctx(), icons::Icon::Chat, theme::ICON_PICKER, muted()));
                    let title = state.title.clone().unwrap_or_else(|| "新会话".to_owned());
                    ui.add(
                        egui::Label::new(egui::RichText::new(title).size(theme::TEXT_META).color(text()))
                            .truncate(),
                    );
                    // The tools, pushed to the far end.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_button(ui, icons::Icon::Close, "收起面板").clicked()
                            || ui.input(|input| input.key_pressed(egui::Key::Escape))
                        {
                            *action = Some(Action::Hide);
                        }
                    });
                });
            });
        })
        .response;
    let handle = ui.interact(bar.rect, ui.id().with("quorfloat-drag"), egui::Sense::drag());
    if handle.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if handle.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
}

/// A square icon button, at the design's size for the top bar.
///
/// @param ui - where to draw.
/// @param icon - which picture.
/// @param tooltip - what it does, shown on hover.
/// @returns the response, so the caller can act on a click.
fn icon_button(ui: &mut egui::Ui, icon: icons::Icon, tooltip: &str) -> egui::Response {
    let size = egui::vec2(theme::ICON_BUTTON, theme::ICON_BUTTON);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let visuals = ui.style().interact(&response);
    if response.hovered() {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(theme::RADIUS_ICON_BUTTON), theme::soft());
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon.chars(),
        theme::font(ui.ctx(), theme::Weight::Regular, theme::ICON),
        visuals.fg_stroke.color,
    );
    response.on_hover_text(tooltip)
}

/// Whatever needs an answer, between the composer and the conversation.
fn cards(ui: &mut egui::Ui, state: &PanelState, action: &mut Option<Action>) {
    if state.interactions.is_empty() {
        return;
    }
    ui.add_space(SECTION_GAP);
    // `auto_shrink` vertically, and bounded: a request is actionable so it has to be on
    // screen, but a year-old card must not push the conversation out of the panel — which
    // is exactly what an `auto_shrink([false, false])` area does even when it is empty.
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
                ui.add_space(SECTION_GAP);
            }
        });
}

/// The height the footer will occupy, before it is drawn.
///
/// Predicted from the design's own padding and type, for the same reason the composer's
/// height is: the conversation is given what is left, and a first frame that guessed would
/// make the panel jump on the second.
///
/// @param ui - the frame, for the item spacing.
/// @returns the strip's height.
fn footer_height(ui: &egui::Ui) -> f32 {
    theme::TEXT_SMALL + 6.0 + ui.spacing().item_spacing.y
}

/// The bottom bar: what the keys do, and what the panel is doing.
fn footer_bar(ui: &mut egui::Ui, state: &PanelState) {
    let frame = egui::Frame::NONE.inner_margin(theme::PAD_FOOTER);
    frame.show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = theme::GAP_TIGHT;
            for (key, what) in [("↵", "发送"), ("⇧↵", "换行"), ("esc", "收起")] {
                ui.label(
                    egui::RichText::new(key)
                        .size(theme::TEXT_SMALL)
                        .color(muted())
                        .background_color(theme::soft()),
                );
                ui.label(theme::meta(ui.ctx(), what));
            }
        });
        // One line, always: the composer's own report while a prompt is in flight, or what
        // the panel is following. Whichever it is, it is the panel's answer to "what is
        // happening", so it lives where the design puts the status.
        let status = state
            .prompt_line
            .clone()
            .unwrap_or_else(|| state.follow.status());
        ui.add(egui::Label::new(theme::meta(ui.ctx(), status)).truncate());
        if let Some(warning) = &state.fonts_warning {
            ui.add(egui::Label::new(egui::RichText::new(warning).size(theme::TEXT_META).color(warn_text())).truncate());
        }
        ui.add(egui::Label::new(theme::meta(ui.ctx(), state.hotkey.clone())).truncate());
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
