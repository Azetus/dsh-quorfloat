//! The composer: the one place the user types, and the one place a turn is stopped.
//!
//! Two details carry most of the weight here.
//!
//! **Enter sends, Shift+Enter breaks the line** — and that is expressed through the
//! widget's own `return_key` rather than by editing text after the fact: the shortcut
//! named there is the one that inserts a newline, so plain Enter is left for us to see
//! and act on. Doing it the other way round (let the newline land, then remove it) gets
//! the cursor position wrong the first time a user edits in the middle.
//!
//! **An Enter that picks an IME candidate must never send.** Chinese input is this
//! panel's normal case, and a half-composed sentence sent to a model is a small disaster
//! that looks like a bug in the model. macOS usually swallows that key before egui sees
//! it, but "usually" is not a guarantee, so a frame carrying a preedit is a frame that
//! cannot submit — and that guard is verified on a real IME rather than assumed
//! (`docs/prototype.md` §26).

use eframe::egui;

use crate::app::PanelState;
use crate::ui::Action;

/// Draw the composer and report what the user asked for.
///
/// @param ui - where to draw.
/// @param state - the state the composer reflects: what was sent, and whether a turn is
///   running.
/// @param draft - the text, owned by the caller so it survives the frame.
/// @param action - where a send or a stop is reported.
pub(super) fn composer(
    ui: &mut egui::Ui,
    state: &PanelState,
    draft: &mut String,
    action: &mut Option<Action>,
) {
    let sending = state.prompt_sending;
    let editor = ui.add_sized(
        [ui.available_width(), EDITOR_HEIGHT],
        egui::TextEdit::multiline(draft)
            .desired_rows(2)
            // Shift+Enter is the combination that inserts a newline; plain Enter is left
            // unconsumed so the check below can turn it into a send.
            .return_key(Some(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter)))
            .hint_text("输入消息…"),
    );

    // Enter sends, unless the user is mid-composition, asking for a line break, or has
    // nothing to send. The decision is a separate function because it is the rule this
    // whole file exists for, and a rule that can only be tested by driving a keyboard is
    // a rule that will not be tested.
    let (enter, shift) = ui.input(|input| (input.key_pressed(egui::Key::Enter), input.modifiers.shift));
    if submits(editor.has_focus(), enter, shift, composing(ui), sending, draft.trim().is_empty()) {
        *action = Some(Action::Send { text: draft.clone() });
    }

    ui.horizontal(|ui| {
        let blank = draft.trim().is_empty();
        let send = ui.add_enabled_ui(!blank && !sending, |ui| {
            ui.add_sized([BUTTON_WIDTH, BUTTON_HEIGHT], egui::Button::new(if sending { "发送中…" } else { "发送" }))
        });
        if send.inner.clicked() {
            *action = Some(Action::Send { text: draft.clone() });
        }
        // Only while there is something to stop: a stop button in an idle conversation is
        // a button that does nothing, which teaches the user to distrust the row.
        if state.turn_active {
            let stop = ui.add_sized([BUTTON_WIDTH, BUTTON_HEIGHT], egui::Button::new("停止"));
            if stop.clicked() {
                *action = Some(Action::Cancel);
            }
        }
    });
    // Reserved whether or not there is anything to say: a line that appears and disappears
    // would move the conversation above it, and the strip's height has already been
    // promised to the layout.
    let line = state.prompt_line.as_deref().unwrap_or_default();
    ui.add_sized(
        [ui.available_width(), STATUS_HEIGHT],
        egui::Label::new(egui::RichText::new(line).size(11.0).color(crate::ui::theme::MUTED)),
    );
}

/// The height the composer will occupy, before it is drawn.
///
/// Computed from constants rather than measured, because the layout needs the number
/// *before* the composer exists: the conversation is given what is left, and a first frame
/// that guesses would either clip the buttons or make the conversation jump on the second.
///
/// @param ui - the frame, for the item spacing.
/// @param state - read only for the status line, whose space is reserved unconditionally.
/// @returns the strip's height.
#[must_use]
pub(super) fn height(ui: &egui::Ui, _state: &PanelState) -> f32 {
    let spacing = ui.spacing().item_spacing.y;
    EDITOR_HEIGHT + spacing + BUTTON_HEIGHT + spacing + STATUS_HEIGHT
}

/// How tall the editor is. Two rows: enough to see a wrapped sentence, small enough that
/// the conversation keeps most of a short panel. Longer text scrolls inside it.
const EDITOR_HEIGHT: f32 = 44.0;

/// How tall the send and stop buttons are, and how wide.
const BUTTON_HEIGHT: f32 = 26.0;
const BUTTON_WIDTH: f32 = 72.0;

/// How tall the line under the buttons is. Always reserved, never conditional.
const STATUS_HEIGHT: f32 = 16.0;

/// Whether an Enter press sends the message.
///
/// Every reason to say no is here rather than spread through the widget code, because this
/// is where the panel decides that a half-typed sentence is not a message:
///
/// - **no focus**: Enter belongs to whatever else the user is doing;
/// - **shift**: the user asked for a line break, and the widget has already inserted one;
/// - **composing**: an IME is choosing characters, and that Enter is picking a candidate;
/// - **sending**: the previous prompt has no verdict yet, and the host would refuse a
///   second one for the same moment;
/// - **blank**: nothing to say, and the host would record an empty turn.
///
/// @returns whether to send.
#[must_use]
fn submits(has_focus: bool, enter: bool, shift: bool, composing: bool, sending: bool, blank: bool) -> bool {
    has_focus && enter && !shift && !composing && !sending && !blank
}

/// Whether an input method is mid-composition.
///
/// A preedit with text in it means the user is choosing characters; the empty preedit the
/// IME sends when it closes does not count, or the composer would refuse to send for one
/// frame after every Chinese word.
///
/// @param ui - the frame's input.
/// @returns whether a composition is in progress.
fn composing(ui: &egui::Ui) -> bool {
    ui.input(|input| {
        input.events.iter().any(|event| {
            matches!(event, egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) if !text.is_empty())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_in_a_focused_composer_sends() {
        assert!(submits(true, true, false, false, false, false));
    }

    #[test]
    fn shift_enter_is_a_line_break_rather_than_a_send() {
        // The widget inserts the newline for this combination; sending as well would post
        // half a sentence and leave the rest in the box.
        assert!(!submits(true, true, true, false, false, false));
    }

    #[test]
    fn an_ime_candidate_enter_never_sends() {
        // The failure this prevents is the one that looks like a model bug: a
        // half-composed Chinese sentence, sent, because the user pressed Enter to pick a
        // candidate. macOS usually swallows that key; "usually" is not a guarantee.
        assert!(!submits(true, true, false, true, false, false));
    }

    #[test]
    fn a_second_enter_while_sending_does_not_send_again() {
        assert!(!submits(true, true, false, false, true, false));
    }

    #[test]
    fn an_empty_composer_has_nothing_to_send() {
        assert!(!submits(true, true, false, false, false, true));
    }

    #[test]
    fn enter_without_focus_belongs_to_something_else() {
        assert!(!submits(false, true, false, false, false, false));
    }

    #[test]
    fn a_preedit_event_means_the_ime_is_composing() {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            events: vec![egui::Event::Ime(egui::ImeEvent::Preedit {
                text: "zhong".to_owned(),
                active_range_chars: None,
            })],
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            assert!(composing(ui), "a non-empty preedit is a composition in progress");
        });
        output.textures_delta.clear();
    }

    #[test]
    fn the_empty_preedit_that_closes_an_ime_is_not_composing() {
        // Otherwise the composer would refuse to send for a frame after every Chinese
        // word, which reads as "the send button sometimes does nothing".
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            events: vec![egui::Event::Ime(egui::ImeEvent::Preedit { text: String::new(), active_range_chars: None })],
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            assert!(!composing(ui), "an empty preedit is the IME closing, not composing");
        });
        output.textures_delta.clear();
    }
}
