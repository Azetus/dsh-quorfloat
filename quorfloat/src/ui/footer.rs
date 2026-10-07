//! What the panel shows about the conversation's *settings*: its statistics, its model and
//! reasoning effort, and its permission preset.
//!
//! Permission sits on the left of the upper strip, a combined model/effort picker on the
//! right, and measured statistics below. Each group receives an allocated rectangle;
//! text is ellipsized before painting, so clipping never disguises a layout overflow.
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
    /// The combined model and effort menu.
    Config,
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

/// Allocate a bounded, single-line picker, including optional effort text.
fn button(ui: &mut egui::Ui, id: egui::Id, icon: Option<Icon>, label: &str, suffix: Option<&str>, marked: bool) -> egui::Response {
    let font = theme::font(ui.ctx(), theme::Weight::Regular, theme::TEXT_META);
    let suffix = suffix.map(|text| {
        let mut job = egui::text::LayoutJob::simple(text.to_owned(), font.clone(), theme::muted(), ui.available_width() / 3.0);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        ui.painter().layout_job(job)
    });
    let suffix_width = suffix.as_ref().map_or(0.0, |text| text.size().x + theme::GAP_CONFIG);
    let icon_width = icon.map_or(0.0, |_| theme::ICON_PICKER + theme::GAP_TIGHT);
    let fixed = theme::FOOTER_PICKER_PAD * 2.0 + icon_width + suffix_width + theme::ICON_CHEVRON + theme::GAP_CONFIG;
    let natural = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), theme::text()).size().x;
    let width = (natural + fixed).min(ui.available_width());
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, theme::FOOTER_PICKER_HEIGHT), egui::Sense::click());
    if response.hovered() || egui::Popup::is_id_open(ui.ctx(), id) {
        ui.painter().rect_filled(rect, theme::RADIUS_PICKER, theme::soft());
    }
    let mut x = rect.left() + theme::FOOTER_PICKER_PAD;
    if let Some(icon) = icon {
        crate::ui::icons::paint(ui, egui::pos2(x + theme::ICON_PICKER / 2.0, rect.center().y), icon, theme::ICON_PICKER,
            if marked { theme::accent() } else { theme::muted() });
        x += icon_width;
    }
    let mut job = egui::text::LayoutJob::simple(label.to_owned(), font, theme::text(), (width - fixed).max(0.0));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let text = ui.painter().layout_job(job);
    ui.painter().galley(egui::pos2(x, rect.center().y - text.size().y / 2.0), text, theme::text());
    if let Some(suffix) = suffix {
        ui.painter().galley(egui::pos2(rect.right() - theme::FOOTER_PICKER_PAD - theme::ICON_CHEVRON - theme::GAP_CONFIG - suffix.size().x,
            rect.center().y - suffix.size().y / 2.0), suffix, theme::muted());
    }
    crate::ui::icons::paint(ui, egui::pos2(rect.right() - theme::FOOTER_PICKER_PAD - theme::ICON_CHEVRON / 2.0, rect.center().y),
        Icon::CaretDown, theme::ICON_CHEVRON, theme::muted());
    response.on_hover_text(label)
}

/// Permission left, model and effort together on the right.
pub(super) fn settings(ui: &mut egui::Ui, state: &PanelState, action: &mut Option<Action>) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, theme::FOOTER_PICKER_HEIGHT), egui::Sense::hover());
    let split = rect.left() + width / 3.0;
    ui.scope_builder(egui::UiBuilder::new().max_rect(egui::Rect::from_min_max(rect.min, egui::pos2(split, rect.bottom())))
        .layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| permissions(ui, state, action));
    ui.scope_builder(egui::UiBuilder::new().max_rect(egui::Rect::from_min_max(egui::pos2(split + theme::GAP_RUNTIME, rect.top()), rect.max))
        .layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| config(ui, state, action));
}

fn effort_label(options: &SessionOptions) -> Option<String> {
    let model = current_model(options)?;
    if model.efforts.is_empty() { return None; }
    Some(options.current_effort.as_ref().map_or_else(|| "默认".to_owned(), |effort|
        model.efforts.iter().find(|option| &option.id == effort).map_or_else(|| effort.clone(), |option| option.name.clone())))
}

fn config(ui: &mut egui::Ui, state: &PanelState, action: &mut Option<Action>) {
    let Some(options) = &state.options else { return };
    if options.models.is_empty() { return; }
    let label = current_model(options).map_or("模型", |model| model.name.as_str());
    let effort = effort_label(options);
    let response = button(ui, Kind::Config.id(), None, label, effort.as_deref(), false);
    egui::Popup::menu(&response).id(Kind::Config.id()).frame(theme::popover_frame())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            ui.set_width(theme::POPOVER_WIDTH);
            for (kind, title, value) in [(Kind::Model, "模型", Some(label)), (Kind::Effort, "推理等级", effort.as_deref())] {
                let Some(value) = value else { continue };
                if let Some(Row::Chosen) = picker::option_row(ui, None, title, Some(value), false, false, false) {
                    egui::Popup::open_id(ui.ctx(), kind.id());
                }
            }
        });
    model(ui, state, action, &response);
    effort_menu(ui, state, action, &response);
}

fn back(ui: &mut egui::Ui, kind: Kind) {
    if ui.button("‹ 返回模型与推理等级").clicked() {
        egui::Popup::close_id(ui.ctx(), kind.id());
        egui::Popup::open_id(ui.ctx(), Kind::Config.id());
    }
}

/// Measured statistics, grouped like the design; no invented token total or absent speed.
fn stat_items(stats: Stats) -> Vec<(Icon, String)> {
    let mut counts = format!("{} 轮 {} 步", stats.turns, stats.steps);
    if let Some(speed) = stats.tokens_per_second { counts += &format!(" · {speed:.0} tok/s"); }
    let mut items = vec![(Icon::Gauge, counts)];
    if let Some(percent) = stats.cache_hit_percent { items.push((Icon::Database, format!("缓存命中 {percent}%"))); }
    if let Some(tokens) = stats.context_tokens {
        let text = match stats.context_limit.filter(|limit| *limit > 0) {
            Some(limit) => format!("上下文 {}%", tokens.saturating_mul(100) / limit),
            None => format!("上下文 {tokens}"),
        };
        items.push((Icon::ChartPie, text));
    }
    items
}

/// A fixed right-aligned group: allocate first, then paint within its rectangle.
pub(super) fn statistics(ui: &mut egui::Ui, stats: Option<Stats>) {
    let Some(stats) = stats else { return };
    let items = stat_items(stats);
    let font = theme::font(ui.ctx(), theme::Weight::Regular, theme::TEXT_SMALL);
    let widths: Vec<f32> = items.iter().map(|(_, text)| ui.painter().layout_no_wrap(text.clone(), font.clone(), theme::muted()).size().x
        + theme::ICON_STAT + theme::GAP_STAT_LABEL).collect();
    let natural = widths.iter().sum::<f32>() + theme::GAP_STATS * (items.len() - 1) as f32;
    let width = natural.min(ui.available_width());
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, theme::FOOTER_INFO_HEIGHT), egui::Sense::hover());
    let budget = (width - theme::GAP_STATS * (items.len() - 1) as f32) / items.len() as f32;
    let mut x = rect.left();
    for ((icon, label), natural_width) in items.into_iter().zip(widths) {
        let item_width = if natural > width { budget } else { natural_width };
        crate::ui::icons::paint(ui, egui::pos2(x + theme::ICON_STAT / 2.0, rect.center().y), icon, theme::ICON_STAT, theme::muted());
        let mut job = egui::text::LayoutJob::simple(label.clone(), font.clone(), theme::muted(),
            (item_width - theme::ICON_STAT - theme::GAP_STAT_LABEL).max(0.0));
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(egui::pos2(x + theme::ICON_STAT + theme::GAP_STAT_LABEL, rect.center().y - galley.size().y / 2.0), galley, theme::muted());
        ui.interact(egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(item_width, rect.height())),
            ui.id().with(icon.name()), egui::Sense::hover()).on_hover_text(label);
        x += item_width + theme::GAP_STATS;
    }
}

/// The model: which one, and a menu of the rest.
///
/// @param ui - where to draw.
/// @param state - for the options and the current model.
/// @param action - where a choice is reported.
fn model(ui: &mut egui::Ui, state: &PanelState, action: &mut Option<Action>, response: &egui::Response) {
    let Some(options) = &state.options else { return };
    let current = options
        .current_model
        .as_ref()
        .and_then(|(provider, id)| options.models.iter().find(|model| &model.provider == provider && &model.id == id));
    let mut chosen: Option<Action> = None;
    egui::Popup::menu(response)
        .open_memory(None)
        .frame(theme::popover_frame())
        .id(Kind::Model.id())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(theme::POPOVER_WIDTH);
            back(ui, Kind::Model);
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
        egui::Popup::close_id(ui.ctx(), Kind::Model.id());
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
fn effort_menu(ui: &mut egui::Ui, state: &PanelState, action: &mut Option<Action>, response: &egui::Response) {
    let Some(options) = &state.options else { return };
    let Some(current) = current_model(options) else { return };
    if current.efforts.is_empty() {
        return;
    }
    let mut chosen = None;
    egui::Popup::menu(response)
        .open_memory(None)
        .frame(theme::popover_frame())
        .id(Kind::Effort.id())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(theme::POPOVER_WIDTH);
            back(ui, Kind::Effort);
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
        egui::Popup::close_id(ui.ctx(), Kind::Effort.id());
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
    let response = button(
        ui,
        Kind::Permission.id(),
        Some(if elevated { Icon::Shield } else { Icon::ShieldCheck }),
        &label,
        None,
        elevated,
    );
    let mut chosen = None;
    egui::Popup::menu(&response)
        .id(Kind::Permission.id())
        .frame(theme::popover_frame())
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
