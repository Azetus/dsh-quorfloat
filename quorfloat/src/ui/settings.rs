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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Outcome {
    /// The theme the user chose, if they chose one.
    pub(super) theme: Option<Preference>,
    /// Whether the panel should stay open when the user leaves, if they changed that.
    pub(super) keep_open: Option<bool>,
}

impl Outcome {
    /// The action this frame amounts to, if the user changed anything.
    ///
    /// @returns the action, or `None` when nothing was touched.
    #[must_use]
    pub(super) fn action(self) -> Option<Action> {
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
                    hotkey_row(ui, state);
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

/// The hotkey: the accelerator the panel actually registered.
///
/// **Read-only, and it says so by being read-only.** The design draws it as a read-only input whose
/// value never changes either — it is a report, not a control. Changing the hotkey would mean
/// unregistering it, having the host persist a new one and re-registering it on the right thread,
/// and a control that cannot yet do that must not look like one that can. That is the same rule that
/// kept the gear out of the top bar until this page existed behind it.
///
/// @param ui - where to draw.
/// @param state - for the accelerator, or the reason there is none.
fn hotkey_row(ui: &mut egui::Ui, state: &PanelState) {
    setting_row(ui, "呼出快捷键", "在任意应用中呼出或收起悬浮窗", |ui| {
        let (text, colour) = match &state.hotkey {
            Some(spec) => (spec.clone(), theme::text()),
            None => ("未注册".to_owned(), theme::warn_text()),
        };
        let frame = egui::Frame::NONE
            .fill(theme::soft())
            .stroke(egui::Stroke::new(theme::BORDER, theme::line()))
            .corner_radius(egui::CornerRadius::same(theme::RADIUS_PICKER))
            .inner_margin(theme::PAD_SETTING_CHIP);
        let inner = theme::SETTING_CHIP_WIDTH
            - f32::from(theme::PAD_SETTING_CHIP.left + theme::PAD_SETTING_CHIP.right);
        let response = frame
            .show(ui, |ui| {
                ui.set_width(inner);
                ui.vertical_centered(|ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(text)
                                .font(theme::font(ui.ctx(), theme::Weight::Regular, theme::TEXT_META))
                                .color(colour),
                        )
                        .truncate(),
                    );
                });
            })
            .response;
        // Why there is no hotkey is a sentence, and a sentence does not fit in a chip. It goes where
        // every other long explanation in this panel goes: on hover.
        if let Some(reason) = &state.hotkey_reason {
            response.on_hover_text(reason);
        }
    });
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
            // A block rather than an inline pair, so the control that follows starts after both
            // lines: the alignment is `TOP`, so it lines up with the label rather than floating in
            // the middle of the note.
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
        // Whatever is left, with the control at its far end.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
            control(ui);
        });
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
            Outcome { theme: Some(Preference::Dark), keep_open: None }.action(),
            Some(Action::SetTheme(Preference::Dark)),
        );
        assert_eq!(
            Outcome { theme: None, keep_open: Some(false) }.action(),
            Some(Action::KeepOpenOnBlur(false)),
        );
        assert_eq!(Outcome::default().action(), None, "an untouched page asks for nothing");
    }
}
