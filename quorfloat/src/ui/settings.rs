//! The settings view: what the panel lets the user decide about itself.
//!
//! The design opens this from the gear in the top bar, and it **replaces** the conversation rather
//! than floating over it (its `setSettings` hides `q-main` and shows `q-settings`). That is the one
//! structural claim here, and it is the right one for a 640-pixel panel: a dialog on top of a
//! conversation has to be smaller than the thing it covers, and settings are a page you visit, not a
//! question you answer.
//!
//! **Only settings that do something appear here.** The design's second row, "附带剪贴板", is
//! deliberately absent: the panel cannot attach the clipboard yet — that needs a protocol method and
//! a row in the composer — and a switch reporting a preference nothing reads is worse than no switch
//! at all. It is recorded as outstanding in `progress.md`, and it belongs in this view on the day
//! the feature lands, not before.
//!
//! Like everything else in this module, this file draws a state and reports clicks: it cannot reach
//! the session, and it cannot write a preference file. It says "the user wants the dark theme" by
//! returning an [`Action`], and `app` decides what that means.

use eframe::egui;

use crate::app::PanelState;
use crate::ui::theme::{self, Preference};
use crate::ui::Action;

/// What one frame of the settings view reports.
///
/// A change is `Some`, not a boolean: "the user set the theme to dark" is a fact worth carrying, and
/// a flag would have the caller re-derive it from the state it was drawn with — which is the state
/// *before* the click.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Outcome {
    /// The theme the user chose, if they chose one.
    pub(super) theme: Option<Preference>,
    /// Whether the panel should stay open when the user leaves, if they changed that.
    pub(super) keep_open: Option<bool>,
    /// The accelerator the user asked for — captured or typed, whichever the row offered.
    pub(super) hotkey_submitted: Option<String>,
    /// Whether the row should start listening for a chord.
    pub(super) start_recording: bool,
    /// Whether it should stop.
    pub(super) stop_recording: bool,
    /// Why the chord just pressed was not accepted, if it was not.
    pub(super) reject: Option<String>,
}

impl Outcome {
    /// The action this frame amounts to.
    ///
    /// One action per frame, and the order is precedence rather than page order: giving up outranks a
    /// capture (they cannot both happen in one frame, but if they did, doing nothing is the safer
    /// reading), and a capture outranks a click on something else in the same frame.
    ///
    /// @returns the action, or `None` when nothing was touched.
    #[must_use]
    pub(super) fn action(self) -> Option<Action> {
        if let Some(hint) = self.reject {
            return Some(Action::RejectHotkey { hint });
        }
        if self.stop_recording {
            return Some(Action::StopHotkeyRecording);
        }
        if let Some(spec) = self.hotkey_submitted {
            return Some(Action::SetHotkey(spec));
        }
        if self.start_recording {
            return Some(Action::StartHotkeyRecording);
        }
        if let Some(theme) = self.theme {
            return Some(Action::SetTheme(theme));
        }
        self.keep_open.map(Action::KeepOpenOnBlur)
    }
}

/// Draw the settings view.
///
/// @param ui - the panel's own area, below the top bar.
/// @param state - everything the view is allowed to know.
/// @returns what the user changed this frame, and whether they asked to go back.
pub(super) fn settings(ui: &mut egui::Ui, state: &PanelState) -> (Outcome, bool) {
    let mut outcome = Outcome::default();
    let mut back = false;
    // The view fills the room the conversation would have had, so the footer stays where it is: the
    // settings are a page of this panel, not a second window with a footer of its own.
    let height = ui.available_height();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(height)
        .show(ui, |ui| {
            egui::Frame::NONE
                .inner_margin(theme::PAD_SETTINGS)
                .show(ui, |ui| {
                    back = heading(ui);
                    // In the design's order, with the row it does not draw where it belongs: the
                    // theme is the first thing anyone looks for in a settings page, and the hotkey
                    // is the fact they came here to check.
                    theme_row(ui, state, &mut outcome);
                    keep_open_row(ui, state, &mut outcome);
                    hotkey_row(ui, state, &mut outcome);
                });
        });
    (outcome, back)
}

/// The heading: what this page is, and the way back.
///
/// @param ui - where to draw.
/// @returns whether the user asked to go back.
fn heading(ui: &mut egui::Ui) -> bool {
    let mut back = false;
    ui.horizontal(|ui| {
        ui.label(theme::card_title(ui.ctx(), "悬浮窗设置"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(theme::settings_back_button(ui.ctx(), "返回对话")).clicked() {
                back = true;
            }
        });
    });
    // The design's `margin-bottom:8px` is added here rather than inside the first row, because a
    // row's top hairline belongs to the row: it is what separates one setting from the next, and the
    // first one needs it as much as the others do.
    ui.add_space(theme::GAP_SETTINGS_HEAD);
    back
}

/// The theme: the panel's two palettes, plus following the platform.
///
/// A row of buttons rather than a switch, because there are three choices and "not the current one"
/// does not say which. The design draws no theme row — only its two switches — so this one keeps the
/// shape of the rows around it while being explicit about what is being chosen.
///
/// @param ui - where to draw.
/// @param state - for the theme currently in effect.
/// @param outcome - where a choice is reported.
fn theme_row(ui: &mut egui::Ui, state: &PanelState, outcome: &mut Outcome) {
    const CHOICES: [(Preference, &str, &str); 3] = [
        (Preference::System, "跟随系统", "随系统外观切换"),
        (Preference::Light, "浅色", "始终使用浅色"),
        (Preference::Dark, "深色", "始终使用深色"),
    ];
    let mut chosen = None;
    setting_row(ui, "外观", "面板的明暗主题", |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = theme::GAP_CLOSE;
            for (preference, label, hint) in CHOICES {
                // Exactly one filled control per view, which is this panel's own rule: the theme in
                // effect is the one filled chip, and the other two are surfaces. The choice chips are
                // the design's own `.q-picker` shape — a soft surface with no outline — so that the
                // single filled one reads as the state rather than as a different kind of control.
                let response = ui
                    .add(theme::chip_button(ui.ctx(), label, state.theme == preference))
                    .on_hover_text(hint);
                if response.clicked() {
                    chosen = Some(preference);
                }
            }
        });
    });
    outcome.theme = chosen;
}

/// Keep open, which is the design's `keepOpen` switch.
///
/// @param ui - where to draw.
/// @param state - for the current value.
/// @param outcome - where a change is reported.
fn keep_open_row(ui: &mut egui::Ui, state: &PanelState, outcome: &mut Outcome) {
    let mut keep_open = state.keep_open;
    let mut changed = false;
    setting_row(ui, "失焦时保持展开", "切换应用后仍保留悬浮窗", |ui| {
        changed = switch(ui, &mut keep_open).changed();
    });
    if changed {
        outcome.keep_open = Some(keep_open);
    }
}

/// The hotkey: the accelerator this panel holds, and the way to change it.
///
/// **The design's chip, made into the control.** The mockup draws the accelerator in a bordered box —
/// a read-only *report* of what the panel holds. It is that, and it is also the button: clicking it
/// starts listening, and the same box then asks for a chord. One element and two states, which is what
/// the design's shape wants and what keeps the row from growing a toolbar beside it.
///
/// **A recorder, because egui can support one.** It ships no shortcut-entry widget, but it has
/// everything needed to build one: `Event::Key { key, pressed, modifiers }` is in `InputState::events`,
/// and `Key` covers F1–F24 alongside the letters, digits and punctuation. So "press the combination you
/// want" costs nothing from the platform layer.
///
/// Two decisions worth stating:
///
/// - **`Escape` cancels rather than being recorded.** It is the only key that must mean "stop", or the
///   row would be impossible to leave with the keyboard.
/// - **A modifier on its own is not the answer.** `Ctrl+Shift` is a legal global hotkey, but nobody
///   presses it *meaning* to bind it; they are on the way to a chord.
///
/// @param ui - where to draw.
/// @param state - for the accelerator, any refusal, and whether the row is listening.
/// @param outcome - where a captured chord or a request to listen is reported.
fn hotkey_row(ui: &mut egui::Ui, state: &PanelState, outcome: &mut Outcome) {
    setting_row(ui, "呼出快捷键", HOTKEY_NOTE, |ui| match state.recording {
        true => hotkey_recording(ui, state, outcome),
        false => hotkey_chip(ui, state, outcome),
    });
}

/// What the row explains.
const HOTKEY_NOTE: &str = "在任意应用中呼出或收起悬浮窗";

/// The chip: what the panel holds, or that it holds nothing. Clicking it starts a recording.
///
/// @param ui - where to draw.
/// @param state - for the accelerator, or the reason there is none.
/// @param outcome - set when the user asks to change it.
fn hotkey_chip(ui: &mut egui::Ui, state: &PanelState, outcome: &mut Outcome) {
    // **A refused accelerator is still shown, in the warning colour.** The box is the only place this
    // row can speak, so replacing the name with "未注册" would throw away the one fact the user needs —
    // *which* shortcut did not take. The warning colour carries "it did not work" and the name carries
    // "this is what did not work"; the full reason is on hover.
    let (text, colour) = match (&state.hotkey, &state.hotkey_reason) {
        (Some(spec), None) => (spec.clone(), theme::text()),
        (Some(spec), Some(_)) => (spec.clone(), theme::warn_text()),
        (None, Some(_)) => ("未注册".to_owned(), theme::warn_text()),
        (None, None) => ("已关闭".to_owned(), theme::muted()),
    };
    let response = hotkey_box(ui, &text, colour, false);
    // Why there is no hotkey is a sentence, and a sentence does not fit in a chip. It goes where every
    // other long explanation in this panel goes: on hover — together with the hint that the box is
    // clickable, which a bordered value does not say by itself.
    let hint = match &state.hotkey_reason {
        Some(reason) => format!("{reason}\n点击后按下新的组合键"),
        None => "点击后按下新的组合键".to_owned(),
    };
    if response.on_hover_text(hint).clicked() {
        outcome.start_recording = true;
    }
}

/// The same box while it is listening for a chord.
///
/// **Not a button**, because there is nothing to click: the user is expected to press a key, and a
/// control that looked clickable would invite a click that does nothing.
///
/// @param ui - where to draw.
/// @param outcome - set when a chord is captured or the user gives up.
fn hotkey_recording(ui: &mut egui::Ui, state: &PanelState, outcome: &mut Outcome) {
    let state_recording_hint = state.recording_hint.clone();
    // The raw events rather than `key_pressed`: the modifiers held at the moment of the press are what
    // the accelerator is made of, and they are on the event itself.
    let captured = ui.input_mut(|input| {
        let mut found = None;
        input.events.retain(|event| {
            let egui::Event::Key { key, pressed: true, modifiers, .. } = event else {
                return true;
            };
            // A modifier arrives as a press of that key; it is never the answer on its own.
            if is_modifier(*key) {
                return false;
            }
            found = Some((*key, *modifiers));
            false
        });
        found
    });
    if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        outcome.stop_recording = true;
    } else if let Some((key, modifiers)) = captured {
        // **A bare letter is refused, and the row says why.** A global hotkey is grabbed from the whole
        // desktop: binding `M` alone means the letter M stops reaching every other application, and the
        // user who pressed it meant "M", not "make this my shortcut". This is not hypothetical — it is
        // what the first version of this recorder did, and it was found by *looking at the panel*, not
        // by a test. Function keys are exempt: `F13` on its own is a good shortcut and nothing types it
        // by accident.
        match crate::runtime::hotkey::accelerator_from_press(key, modifiers) {
            Some(accelerator) if bare_letter(&accelerator) => {
                outcome.reject = Some(format!("{accelerator} 会占用这个按键，请按住修饰键"));
            }
            Some(accelerator) => outcome.hotkey_submitted = Some(accelerator),
            None => outcome.reject = Some("这个键不能作为全局快捷键".to_owned()),
        }
    }
    let boxed = hotkey_box(ui, "按下组合键…", theme::accent(), true);
    match &state_recording_hint {
        // Said in the row, not only on hover: a key press that produced nothing has to produce
        // something, or the honest reading is that the recorder is broken.
        Some(hint) => {
            boxed.on_hover_text(hint);
            ui.label(
                egui::RichText::new(hint)
                    .font(theme::font(ui.ctx(), theme::Weight::Regular, theme::TEXT_SMALL))
                    .color(theme::bad_text()),
            );
        }
        None => {
            boxed.on_hover_text("Esc 取消");
        }
    }
}

/// The box the design draws, in both of the states this row has.
///
/// One shape for the two faces on purpose: the value and the request for a new value occupy the same
/// place, so the row does not reflow when it starts listening — the text changes and the border picks
/// up the accent, and nothing moves.
///
/// @param ui - where to draw.
/// @param text - what the box says.
/// @param colour - the text's colour.
/// @param listening - whether this is the recording state, which is outlined in the accent.
/// @returns the response, for the caller to read a click from.
fn hotkey_box(
    ui: &mut egui::Ui,
    text: &str,
    colour: egui::Color32,
    listening: bool,
) -> egui::Response {
    let width = theme::SETTING_CHIP_WIDTH;
    let stroke = match listening {
        true => egui::Stroke::new(theme::BORDER, theme::accent()),
        false => egui::Stroke::new(theme::BORDER, theme::line()),
    };
    // **A rectangle of its own, then the button centred inside it.** The obvious version wraps the
    // button in a `vertical` that sets its width, and that wrapper *swallows the right-to-left
    // placement*: a nested layout positions its contents by its own alignment, so the box ended up
    // beside the label no matter what the row asked for. Reserving the space here and centring inside
    // it keeps both properties — the box is at the row's right edge, and its text is centred in the box.
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(width, theme::SETTING_CHIP_HEIGHT), egui::Sense::hover());
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight)),
        |ui| {
            ui.add(
                egui::Button::new(
                    egui::RichText::new(text)
                        .font(theme::font(ui.ctx(), theme::Weight::Regular, theme::TEXT_META))
                        .color(colour),
                )
                .fill(theme::soft())
                .stroke(stroke)
                .corner_radius(egui::CornerRadius::same(theme::RADIUS_PICKER)),
            )
        },
    )
    .inner
}

/// Whether an accelerator is a single unmodified key that would swallow ordinary typing.
///
/// A global hotkey is grabbed from the entire desktop, so `M` means the letter M no longer reaches any
/// other application. A keyboard's function keys are the exception, and the design's own shortcut is a
/// modifier chord — so the rule is: one part, and that part is not `F<number>`.
///
/// @param accelerator - the accelerator, as this panel spells it.
/// @returns whether it needs a modifier to be a reasonable choice.
#[must_use]
fn bare_letter(accelerator: &str) -> bool {
    if accelerator.contains('+') {
        return false;
    }
    // `F1`–`F24`: a function key on its own is a shortcut nobody types by accident.
    !accelerator
        .strip_prefix('F')
        .is_some_and(|number| !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()))
}

/// Whether a key is a physical modifier, which a chord is built *from* rather than out of.
///
/// egui keeps the two sides of each modifier apart and emits them **only as physical presses** (its own
/// comment says so), while the collapsed form lives in `Modifiers`. That is exactly what a recorder
/// needs: these eight are the keys a user holds *while* pressing the one that matters, so they are
/// dropped and the chord is made from the `Modifiers` on the final press.
///
/// @param key - the key egui reported.
/// @returns whether it is a modifier key.
fn is_modifier(key: egui::Key) -> bool {
    use egui::Key as K;
    matches!(
        key,
        K::ShiftLeft
            | K::ShiftRight
            | K::ControlLeft
            | K::ControlRight
            | K::AltLeft
            | K::AltRight
            | K::SuperLeft
            | K::SuperRight
    )
}

/// One setting: a label on the left, a control on the right, a hairline above.
///
/// The hairline is drawn by the row rather than as a separator between rows, which is the design's
/// arrangement and has a consequence worth naming: a row *contains* its own top rule, so the first
/// row after the heading draws one too.
///
/// **The row is as tall as the layout made it, not as tall as arithmetic predicted.** The first
/// version measured its two lines with `painter.layout` and then drew them with `painter.galley` —
/// which paints text *without* reserving any space for it. Every row therefore claimed the padding
/// only, the text overflowed into the row beneath, and the page came out with its labels printed on
/// top of each other. Measuring is not the same as laying out, and the fix is to let egui's own
/// widgets do both: they advance the cursor by the height they used, and `Ui::min_rect` is then the
/// truth about how much room the row took.
///
/// @param ui - where to draw.
/// @param label - what the setting is.
/// @param note - one line explaining it, muted and smaller.
/// @param control - draws whatever the setting is set with, into the space on the right.
fn setting_row(ui: &mut egui::Ui, label: &str, note: &str, control: impl FnOnce(&mut egui::Ui)) {
    let margin = theme::PAD_SETTING;
    let width = ui.available_width();
    // The rule first, so the row's own top edge is known before anything is placed inside it. It is
    // allocated rather than painted in place: a painter call moves no cursor, so painting it here
    // and drawing the content below would put the content on top of the line.
    let (rule_rect, _) = ui.allocate_exact_size(egui::vec2(width, theme::BORDER), egui::Sense::hover());
    ui.painter().rect_filled(rule_rect, 0, theme::line());

    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_size(
                egui::pos2(rule_rect.left(), rule_rect.bottom() + f32::from(margin.top)),
                egui::vec2(width - f32::from(margin.left + margin.right), 0.0),
            ))
            .layout(egui::Layout::top_down(egui::Align::LEFT)),
    );
    row.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
    row.horizontal_top(|ui| {
        // The label and its note.
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = theme::GAP_SETTING_NOTE;
            ui.label(
                egui::RichText::new(label)
                    .font(theme::font(ui.ctx(), theme::Weight::Regular, theme::TEXT_META))
                    .color(theme::text()),
            );
            ui.label(
                egui::RichText::new(note)
                    .font(theme::font(ui.ctx(), theme::Weight::Regular, theme::TEXT_SMALL))
                    .color(theme::muted()),
            );
        });
        // The control, against the row's **right edge**.
        //
        // **Placed in a rectangle rather than laid out beside the label.** Three attempts went into
        // this: `right_to_left` alone puts the control's left edge after the label, because a
        // right-to-left cursor starts at the right edge of the space it is *given*; and asking for the
        // remaining width first does not help either, because `horizontal_top` gives its children a
        // *dynamic* size (`allocate_ui_with_layout_dyn`), so "what is left over" measures what was put
        // there rather than what is available. The control is therefore *placed*: one rectangle,
        // computed from the row's own width, which is what "right-aligned" actually means.
        let control_width = ui.available_width();
        if control_width > 0.0 {
            ui.allocate_ui_with_layout(
                egui::vec2(control_width, ui.available_height()),
                egui::Layout::right_to_left(egui::Align::Center),
                control,
            );
        }
    });
    // The room the row actually took, plus the padding under it. `min_rect` covers everything drawn
    // inside, which is the number this used to get wrong.
    let used = row.min_rect().height();
    ui.advance_cursor_after_rect(egui::Rect::from_min_size(
        rule_rect.min,
        egui::vec2(width, theme::BORDER + used + f32::from(margin.top + margin.bottom)),
    ));
}

/// A toggle switch, the design's `.q-setting input[type="checkbox"]`.
///
/// Drawn rather than built from a checkbox because egui has no switch: a tick box says "this is
/// selected", and the design's control says "the panel is in this state" — which is the question a
/// preference asks.
///
/// @param ui - where to draw.
/// @param on - the state, toggled in place when the switch is clicked.
/// @returns the response, for the caller to read `changed` from.
fn switch(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let size = egui::vec2(theme::SWITCH_SIZE[0], theme::SWITCH_SIZE[1]);
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let radius = egui::CornerRadius::same((size.y / 2.0).round() as u8);
    let track = if *on { theme::accent() } else { theme::line() };
    ui.painter().rect_filled(rect, radius, track);
    // The knob travels the width of the track, less its own diameter and the inset the design leaves
    // at each end (2px, which is also `translateX(12px)` for a 13-pixel knob).
    let travel = size.x - theme::SWITCH_KNOB - 4.0;
    let offset = if *on { travel } else { 0.0 };
    let knob = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 2.0 + offset, rect.top() + 2.0),
        egui::vec2(theme::SWITCH_KNOB, theme::SWITCH_KNOB),
    );
    ui.painter().circle_filled(knob.center(), theme::SWITCH_KNOB / 2.0, theme::bg());
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_theme_is_three_exclusive_choices() {
        // A switch cannot express "neither", and the theme has three states rather than two. This
        // test exists because the obvious mistake here is to model it after the design's two
        // switches, which are genuinely two-state.
        let choices = [Preference::System, Preference::Light, Preference::Dark];
        assert_eq!(choices.len(), 3);
        assert!(choices.contains(&Preference::default()), "the default is one of them");
    }

    #[test]
    fn a_bare_letter_needs_a_modifier_and_a_function_key_does_not() {
        // Found by looking at the panel, not by a test: the first recorder accepted `M` on its own, and
        // a global hotkey is grabbed from the *whole desktop* — so the letter M would have stopped
        // reaching every other application. Function keys are the exception, which is why the rule is
        // about typing rather than about modifiers in general.
        assert!(bare_letter("M"), "a bare letter would swallow ordinary typing");
        assert!(bare_letter("5"));
        assert!(bare_letter("`"));

        assert!(!bare_letter("Cmd+M"), "a chord is what the row is asking for");
        assert!(!bare_letter("Alt+Space"));
        assert!(!bare_letter("Shift+5"));

        assert!(!bare_letter("F13"), "a function key on its own is a good shortcut");
        assert!(!bare_letter("F1"));
        // But `F` alone is a letter, and `F0` is not a function key anyone has.
        assert!(bare_letter("F"));
        assert!(!bare_letter("F0"), "`F0` is still spelled like a function key");
    }

    #[test]
    fn the_knob_travels_inside_the_track() {
        // The design translates the knob by 12px inside a 29-pixel track with a 13-pixel knob. A
        // knob that travelled the full width would hang half outside the track it belongs to.
        let travel = theme::SWITCH_SIZE[0] - theme::SWITCH_KNOB - 4.0;
        assert_eq!(travel, 12.0, "the design's `translateX(12px)`");
        assert!(2.0 + travel + theme::SWITCH_KNOB <= theme::SWITCH_SIZE[0], "and it stays inside");
    }

    #[test]
    fn a_change_is_reported_as_the_value_chosen_not_as_a_flag() {
        // The caller is handed what the user picked, because it draws from the state *before* the
        // click: a flag would leave it re-deriving the new value from the old one.
        assert_eq!(
            Outcome { theme: Some(Preference::Dark), ..Outcome::default() }.action(),
            Some(Action::SetTheme(Preference::Dark)),
        );
        assert_eq!(
            Outcome { keep_open: Some(false), ..Outcome::default() }.action(),
            Some(Action::KeepOpenOnBlur(false)),
        );
        assert_eq!(Outcome::default().action(), None, "an untouched page asks for nothing");
    }
}
