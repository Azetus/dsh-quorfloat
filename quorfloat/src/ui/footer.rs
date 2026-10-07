//! What the panel shows about the conversation's *settings*: its statistics, its model and
//! reasoning effort, and its permission preset.
//!
//! **Function first.** The design draws all three in the bottom strip of the window, and their
//! final shape is a later pass; what this file is for is making each of them do the thing it exists
//! to do — read a value from the host, offer the alternatives, and report the choice. The geometry
//! here is therefore deliberately plain: the pickers reuse the rows the workspace menu already had
//! (`ui/picker.rs`), so the vocabulary is not new even where the arrangement is.
//!
//! Two rules carried over from the rest of the panel, because they are what makes a strip of
//! controls honest rather than decorative:
//!
//! - **Nothing is invented.** A value the host has not reported is not drawn as a default; a picker
//!   with no options to offer is not drawn at all. An empty menu reads as a failure, and a made-up
//!   model name reads as a fact.
//! - **A refusal is shown where the value is.** A permission that did not change leaves the old
//!   value on screen, which is indistinguishable from a click that did nothing — so the reason
//!   travels in the panel state and is drawn beside these controls.

use eframe::egui;

use crate::app::PanelState;
use crate::app::session::{ModelOption, SessionOptions, Stats};
use crate::ui::icons::Icon;
use crate::ui::picker::{self, Row};
use crate::ui::{Action, theme};

/// Which footer popup is open, for the development switch in `ui/mod.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// The model list.
    Model,
    /// The reasoning-effort list.
    Effort,
    /// The permission list.
    Permission,
}

impl Kind {
    /// The id the popup's open state is remembered under.
    fn id(self) -> egui::Id {
        egui::Id::new(("quorfloat-footer", self as u8))
    }
}

/// The popup id of one footer picker, for the development switch.
///
/// @param kind - which picker.
/// @returns the id its open state lives under.
pub(crate) fn popup_id(kind: Kind) -> egui::Id {
    kind.id()
}

/// Draw the three settings controls, right to left.
///
/// Right to left because that is how the design's strip reads: the model is the widest label and
/// sits at the far right, with the effort beside it and the permission at the left of the group.
///
/// @param ui - where to draw; a right-to-left layout.
/// @param state - for the options, the current values, and any refusal.
/// @param action - where a choice is reported.
pub(super) fn settings(ui: &mut egui::Ui, state: &PanelState, action: &mut Option<Action>) {
    permissions(ui, state, action);
    effort(ui, state, action);
    model(ui, state, action);
}

/// The statistics: turns, steps, output speed, cache share and context occupancy.
///
/// **Only what was measured.** Every figure is optional and an absent one is left out rather than
/// drawn as zero: "0 tok/s" reads as a stalled model, and a session that has not finished a step has
/// no speed at all — which is the normal state of a panel somebody has just opened.
///
/// @param ui - where to draw.
/// @param stats - what the host reported, if anything.
pub(super) fn statistics(ui: &mut egui::Ui, stats: Option<Stats>) {
    let Some(stats) = stats else { return };
    ui.spacing_mut().item_spacing.x = theme::GAP_TIGHT;
    // The counts are the figures the web UI's own strip leads with, in its wording ("3 轮 7 步").
    let counts = format!("{} 轮 {} 步", stats.turns, stats.steps);
    ui.label(theme::meta(ui.ctx(), &counts));
    if let Some(speed) = stats.tokens_per_second {
        chip(ui, &format!("{speed:.0} tok/s"));
    }
    if let Some(percent) = stats.cache_hit_percent {
        chip(ui, &format!("缓存命中 {percent}%"));
    }
    if let Some(tokens) = stats.context_tokens {
        // A share when the capacity is known, the raw count when it is not: a percentage of an
        // unknown total is not a number anybody can act on.
        match stats.context_limit.filter(|limit| *limit > 0) {
            Some(limit) => chip(ui, &format!("上下文 {}%", tokens.saturating_mul(100) / limit)),
            None => chip(ui, &format!("上下文 {tokens}")),
        }
    }
}

/// One small figure, with the same muted treatment as the counts beside it.
///
/// @param ui - where to draw.
/// @param text - the figure.
fn chip(ui: &mut egui::Ui, text: &str) {
    // `_` separators are for the eye: a six-digit token count is unreadable as one run.
    ui.label(theme::meta(ui.ctx(), &text.replace('_', "")));
}

/// The model: which one, and a menu of the rest.
///
/// @param ui - where to draw.
/// @param state - for the options and the current model.
/// @param action - where a choice is reported.
fn model(ui: &mut egui::Ui, state: &PanelState, action: &mut Option<Action>) {
    let Some(options) = &state.options else { return };
    let current = options
        .current_model
        .as_ref()
        .and_then(|(provider, id)| options.models.iter().find(|model| &model.provider == provider && &model.id == id));
    let label = current.map_or_else(|| "模型".to_owned(), |model| model.name.clone());
    let response = picker::picker_button(ui, Kind::Model.id(), Some(Icon::Chat), &label, false);
    // The model and its effort live in one menu, because the host holds them in one selection: an
    // effort chosen without its model would be a value the model may not accept.
    let mut chosen: Option<Action> = None;
    egui::Popup::menu(&response)
        .id(Kind::Model.id())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(theme::POPOVER_WIDTH);
            picker::popover_head(ui, "模型", "");
            if options.models.is_empty() {
                picker::popover_note(ui, "宿主没有提供可选模型");
                return;
            }
            for model in &options.models {
                if let Some(Row::Chosen) = picker::option_row(
                    ui,
                    None,
                    &model.name,
                    Some(&model.provider),
                    current.is_some_and(|current| current.id == model.id),
                    false,
                    false,
                ) {
                    chosen = Some(Action::SelectModel {
                        provider: model.provider.clone(),
                        model: model.id.clone(),
                        // The effort travels with the model: the host resets it to the model's own
                        // default otherwise, which is not what a user who has set one expects.
                        effort: effort_for(options, model),
                    });
                }
            }
        });
    if let Some(chosen) = chosen {
        *action = Some(chosen);
    }
}

/// Which effort to send when a model is chosen.
///
/// The one in effect when the new model accepts it, otherwise the model's own default, otherwise
/// nothing — and "nothing" is a real answer: it means the host decides, which is what a model with
/// no reasoning control wants.
///
/// @param options - the host's answer, for the effort in effect.
/// @param model - the model being chosen.
/// @returns the effort to send, if any.
fn effort_for(options: &SessionOptions, model: &ModelOption) -> Option<String> {
    let current = options.current_effort.as_ref();
    if let Some(current) = current.filter(|effort| model.efforts.iter().any(|option| &option.id == *effort)) {
        return Some(current.clone());
    }
    model
        .default_effort
        .clone()
        .filter(|effort| model.efforts.iter().any(|option| &option.id == effort))
}

/// The reasoning effort, when the current model has any.
///
/// Absent for a model with no reasoning control, which is most of them: a control that opens an
/// empty menu is worse than no control.
///
/// @param ui - where to draw.
/// @param state - for the options.
/// @param action - where a choice is reported.
fn effort(ui: &mut egui::Ui, state: &PanelState, action: &mut Option<Action>) {
    let Some(options) = &state.options else { return };
    let Some(current) = current_model(options) else { return };
    if current.efforts.is_empty() {
        return;
    }
    let label = match &options.current_effort {
        Some(effort) => current
            .efforts
            .iter()
            .find(|option| &option.id == effort)
            .map_or_else(|| effort.clone(), |option| option.name.clone()),
        // No effort set: the model's own default is what the host will use, and saying so is more
        // honest than showing an empty control.
        None => "默认".to_owned(),
    };
    let response = picker::picker_button(ui, Kind::Effort.id(), Some(Icon::Brain), &label, false);
    let mut chosen = None;
    egui::Popup::menu(&response)
        .id(Kind::Effort.id())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(theme::POPOVER_WIDTH);
            picker::popover_head(ui, "思考档位", &current.name);
            for option in &current.efforts {
                if let Some(Row::Chosen) =
                    picker::option_row(ui, None, &option.name, None, Some(&option.id) == options.current_effort.as_ref(), false, false)
                {
                    chosen = Some(Action::SelectEffort { effort: option.id.clone() });
                }
            }
        });
    if let Some(chosen) = chosen {
        *action = Some(chosen);
    }
}

/// The permission preset.
///
/// @param ui - where to draw.
/// @param state - for the options and the current preset.
/// @param action - where a choice is reported.
fn permissions(ui: &mut egui::Ui, state: &PanelState, action: &mut Option<Action>) {
    let Some(options) = &state.options else { return };
    if options.permissions.is_empty() {
        return;
    }
    let label = options
        .current_permission
        .as_ref()
        .and_then(|value| options.permissions.iter().find(|option| &option.value == value))
        .map_or_else(|| "权限".to_owned(), |option| option.name.clone());
    // A shield that is filled when the preset is not the narrowest one: the whole point of this
    // control is that the panel can be running with more access than the user remembers granting.
    let elevated = options
        .current_permission
        .as_deref()
        .is_some_and(|value| value != options.permissions[0].value);
    let response = picker::picker_button(
        ui,
        Kind::Permission.id(),
        Some(if elevated { Icon::Shield } else { Icon::ShieldCheck }),
        &label,
        elevated,
    );
    let mut chosen = None;
    egui::Popup::menu(&response)
        .id(Kind::Permission.id())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(theme::POPOVER_WIDTH);
            picker::popover_head(ui, "会话权限", "");
            for option in &options.permissions {
                if let Some(Row::Chosen) = picker::option_row(
                    ui,
                    None,
                    &option.name,
                    None,
                    Some(&option.value) == options.current_permission.as_ref(),
                    false,
                    false,
                ) {
                    chosen = Some(Action::SetPermission { value: option.value.clone() });
                }
            }
            picker::popover_note(ui, "权限决定沙箱范围与审批策略");
        });
    if let Some(chosen) = chosen {
        *action = Some(chosen);
    }
}

/// The model in effect, if the options name one that is offered.
///
/// @param options - the host's answer.
/// @returns the model, or `None` when the host has not said which one is in use.
fn current_model(options: &SessionOptions) -> Option<&ModelOption> {
    let (provider, id) = options.current_model.as_ref()?;
    options
        .models
        .iter()
        .find(|model| &model.provider == provider && &model.id == id)
}
