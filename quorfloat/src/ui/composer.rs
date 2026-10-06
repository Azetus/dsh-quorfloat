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
    use crate::ui::theme;

    let sending = state.prompt_sending;
    let frame = egui::Frame::NONE.inner_margin(theme::PAD_COMPOSER);
    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = theme::GAP_COMPOSER;
            // The design's leading mark: what this row is for, in the one colour that means
            // "here". It is not a button and does not pretend to be one.
            let mark = theme::TEXT_COMPOSER * 0.8;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(mark, mark), egui::Sense::hover());
            crate::ui::icons::paint(ui, rect.center(), crate::ui::icons::Icon::Search, mark, theme::accent());

            let editor_width = (ui.available_width() - theme::SEND_BUTTON - theme::GAP_COMPOSER).max(80.0);
            let editor = ui.add_sized(
                [editor_width, editor_height(draft)],
                egui::TextEdit::multiline(draft)
                    .desired_rows(1)
                    // The panel's largest text, on purpose: this is the one place the user
                    // is looking, and the design sets it at 20px against a 14px body.
                    .font(theme::font(ui.ctx(), theme::Weight::Regular, theme::TEXT_COMPOSER))
                    // Shift+Enter is the combination that inserts a newline; plain Enter is
                    // left unconsumed so the check below can turn it into a send.
                    .return_key(Some(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter)))
                    // No box of its own: the design's input is a bare line on the panel, and
                    // egui's default would paint it `extreme_bg_color` — a near-black
                    // rectangle in the middle of a light panel.
                    .frame(egui::Frame::NONE)
                    // The placeholder is where a user looks when nothing happens, so it says
                    // the one thing that would stop a send: there is no conversation yet.
                    .hint_text(if state.attached.is_some() {
                        "问点什么…"
                    } else if state.creating_on_submit {
                        // Sending will start a conversation: said here because the alternative
                        // is a user wondering whether the panel is attached to anything.
                        "问点什么…（发送时新建会话）"
                    } else {
                        "正在打开会话…"
                    }),
            );

            // Enter sends, unless the user is mid-composition, asking for a line break, or
            // has nothing to send. The decision is a separate function because it is the
            // rule this whole file exists for, and a rule that can only be tested by driving
            // a keyboard is a rule that will not be tested.
            let (enter, shift) = ui.input(|input| (input.key_pressed(egui::Key::Enter), input.modifiers.shift));
            if submits(editor.has_focus(), enter, shift, composing(ui), sending, draft.trim().is_empty()) {
                *action = Some(Action::Send { text: draft.clone() });
            }

            // One button, two meanings — send, or stop what is being generated. The design
            // swaps its icon rather than showing both, which is also the honest thing: at any
            // moment only one of the two is what the user wants.
            let stop = state.turn_active;
            // Sending needs somewhere to send to. The button says so by being disabled, and a
            // hover explains why — a button that looks ready and silently refuses is how "my
            // input will not send" becomes a bug report about the input.
            // Sending is possible when there is somewhere to send to, or when sending is what
            // creates it. The only state that cannot send is the one in between: a conversation
            // is pinned but not attached yet.
            let ready = state.attached.is_some() || state.creating_on_submit;
            let enabled = stop || (ready && !draft.trim().is_empty());
            let refusal = (!ready).then_some("正在打开会话…");
            if submit_button(ui, stop, enabled, refusal).clicked() {
                if stop {
                    *action = Some(Action::Cancel);
                } else if !draft.trim().is_empty() {
                    *action = Some(Action::Send { text: draft.clone() });
                }
            }
        });
    });
}

/// The round-square button at the end of the composer.
///
/// @param ui - where to draw.
/// @param stop - whether a turn is in flight, which turns the send into a stop.
/// @param enabled - whether there is anything to do.
/// @returns the response, so the caller can act on a click.
fn submit_button(ui: &mut egui::Ui, stop: bool, enabled: bool, refusal: Option<&str>) -> egui::Response {
    use crate::ui::theme;

    let size = egui::vec2(theme::SEND_BUTTON, theme::SEND_BUTTON);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let hovering = response.hovered() && enabled;
    let (fill, foreground) = match (enabled, hovering) {
        (false, _) => (theme::soft(), theme::muted()),
        (true, false) => (theme::button(), theme::on_button()),
        (true, true) => (theme::accent(), theme::bg()),
    };
    ui.painter().rect_filled(rect, egui::CornerRadius::same(theme::RADIUS_SUBMIT), fill);
    let icon = if stop { crate::ui::icons::Icon::Stop } else { crate::ui::icons::Icon::ArrowUp };
    crate::ui::icons::paint(ui, rect.center(), icon, theme::ICON, foreground);
    if enabled {
        response.on_hover_text(if stop { "停止生成" } else { "发送" })
    } else if let Some(refusal) = refusal {
        response.on_hover_text(refusal)
    } else {
        response
    }
}

/// How tall the editor is for a given draft.
///
/// The design starts the input at a single line (38px) and lets it grow, which is the one
/// behaviour a composer needs: a wrapped sentence the user cannot see is a sentence they
/// will send by accident. Growth is counted in *explicit* lines — a paragraph that wraps
/// keeps its height and scrolls instead, because measuring wrapped text needs the font and
/// the width, and this number is needed by the layout before either is laid out. That trade
/// is stated here rather than discovered later.
///
/// @param draft - what the user has typed.
/// @returns the height to give the editor, between the design's minimum and its cap.
#[must_use]
fn editor_height(draft: &str) -> f32 {
    let rows = draft.lines().count().max(1) as f32;
    (rows * crate::ui::theme::LINE_COMPOSER + EDITOR_PADDING)
        .clamp(EDITOR_MIN_HEIGHT, EDITOR_MAX_HEIGHT)
}

/// The height the editor starts at, from the design's `height:38px`.
const EDITOR_MIN_HEIGHT: f32 = 38.0;

/// The tallest the editor grows before it scrolls, from the design's own cap.
const EDITOR_MAX_HEIGHT: f32 = 120.0;

/// What `TextEdit` adds around its text, which the row maths has to account for.
const EDITOR_PADDING: f32 = 8.0;

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
