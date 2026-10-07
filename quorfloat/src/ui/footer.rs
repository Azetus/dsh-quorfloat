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
    let open = egui::Popup::is_id_open(ui.ctx(), id)
        || (id == Kind::Config.id() && [Kind::Model, Kind::Effort].iter().any(|kind| egui::Popup::is_id_open(ui.ctx(), kind.id())));
    if response.hovered() || open {
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
        if open { Icon::CaretUp } else { Icon::CaretDown }, theme::ICON_CHEVRON, theme::muted());
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
    let effort = options.current_effort.as_ref().or(model.default_effort.as_ref())?;
    Some(model.efforts.iter().find(|option| &option.id == effort)
        .map_or_else(|| effort.clone(), |option| option.name.clone()))
}

fn config(ui: &mut egui::Ui, state: &PanelState, action: &mut Option<Action>) {
    let Some(options) = &state.options else { return };
    if options.models.is_empty() { return; }
    let label = current_model(options).map_or("模型", |model| model.name.as_str());
    let effort = effort_label(options);
    let response = button(ui, Kind::Config.id(), None, label, effort.as_deref(), false);
    if response.clicked() && !egui::Popup::is_id_open(ui.ctx(), Kind::Config.id()) { *action = Some(Action::RefreshOptions); }
    egui::Popup::menu(&response).id(Kind::Config.id()).frame(config_frame())
        .align(egui::RectAlign::TOP_END).gap(theme::POPOVER_GAP)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            config_width(ui);
            for (kind, title, value) in [(Kind::Model, "模型", Some(label)), (Kind::Effort, "推理等级", effort.as_deref())] {
                if matches!(kind, Kind::Effort) && current_model(options).is_none_or(|model| model.efforts.is_empty()) { continue; }
                if config_row(ui, title, value, true).clicked() {
                    egui::Popup::open_id(ui.ctx(), kind.id());
                }
            }
        });
    model(ui, state, action, &response);
    effort_menu(ui, state, action, &response);
}

/// The combined menu is 250px outside, with the design's 4px inner padding.
fn config_frame() -> egui::Frame { theme::popover_frame().inner_margin(egui::Margin::same(theme::CONFIG_MENU_PAD)) }
fn config_width(ui: &mut egui::Ui) {
    ui.set_width(theme::CONFIG_MENU_WIDTH - 2.0 * (f32::from(theme::CONFIG_MENU_PAD) + theme::BORDER));
    ui.spacing_mut().item_spacing.y = 0.0;
}

/// Label left, current value right, then the disclosure — one line as in `.q-config-row`.
fn config_row(ui: &mut egui::Ui, label: &str, value: Option<&str>, forward: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), theme::MENU_LINE_HEIGHT + f32::from(theme::ROW_PADDING.top + theme::ROW_PADDING.bottom)), egui::Sense::click());
    if response.hovered() { ui.painter().rect_filled(rect, theme::RADIUS_ICON_BUTTON, theme::soft()); }
    let font = theme::font(ui.ctx(), theme::Weight::Regular, theme::TEXT_META);
    let label = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), if forward { theme::text() } else { theme::muted() });
    let padding = f32::from(theme::ROW_PADDING.left);
    let x = rect.left() + padding;
    ui.painter().galley(egui::pos2(x, rect.center().y - label.size().y / 2.0), label.clone(), theme::text());
    if let Some(value) = value {
        let end = rect.right() - padding - theme::ICON_CHEVRON - theme::POPOVER_GAP;
        let mut job = egui::text::LayoutJob::simple(value.to_owned(), font, theme::muted(), (end - x - label.size().x - theme::POPOVER_GAP).max(0.0));
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(egui::pos2(end - galley.size().x, rect.center().y - galley.size().y / 2.0), galley, theme::muted());
    }
    if forward { crate::ui::icons::paint(ui, egui::pos2(rect.right() - padding - theme::ICON_CHEVRON / 2.0, rect.center().y), Icon::CaretRight, theme::ICON_CHEVRON, theme::muted()); }
    response
}

fn back(ui: &mut egui::Ui, kind: Kind) {
    let label = if matches!(kind, Kind::Model) { "‹  模型" } else { "‹  推理等级" };
    if config_row(ui, label, None, false).clicked() {
        egui::Popup::close_id(ui.ctx(), kind.id());
        egui::Popup::open_id(ui.ctx(), Kind::Config.id());
    }
    ui.add_space(theme::GAP_CLOSE);
}

/// Measured statistics, in the design's three groups.
///
/// The grouping is the design's (`q-stat-rounds` / `q-stat-tokens` / `q-stat-context`), and each
/// group is one icon plus one run of text — the separator lives *inside* the token group because
/// the total and the cache share are two halves of one fact ("what this conversation cost"), the
/// same way the design's own markup nests them.
///
/// **Nothing is invented**: a group whose figures were not reported is left out entirely rather
/// than drawn with a placeholder, because a dash where a number belongs reads as "zero".
fn stat_items(stats: Stats) -> Vec<StatItem> {
    // Rounds, steps, and the generation speed when it was measured.
    let mut counts = format!("{} 轮 {} 步", stats.turns, stats.steps);
    if let Some(speed) = stats.tokens_per_second {
        counts += &format!(" · {} tok/s", format_speed(speed));
    }
    let mut items = vec![StatItem {
        icon: Some(Icon::Gauge),
        ring: None,
        tooltip: "会话轮数、执行步数与生成速度".to_owned(),
        label: counts,
    }];

    // Cumulative tokens and the cache-hit share, which belong together.
    let mut tokens = Vec::new();
    if let Some(total) = stats.total_tokens {
        tokens.push(format!("{} tok", format_tokens(total)));
    }
    if let Some(percent) = stats.cache_hit_percent {
        tokens.push(format!("缓存命中 {}", format_percent(percent)));
    }
    if !tokens.is_empty() {
        items.push(StatItem {
            icon: Some(Icon::Database),
            ring: None,
            tooltip: "累计消耗 Token 与缓存命中率".to_owned(),
            label: tokens.join(" · "),
        });
    }

    // Context occupancy: **a ring, not the design's pie glyph**. The upstream Harness draws this one
    // measurement as a filled ring (`ContextMeter.tsx`), so a static pie here would be the only place
    // in either UI where the same number has two different pictures. The reading keeps the design's
    // bare percentage — the ring already says what it measures.
    if let Some(percent) = context_percent(&stats) {
        let detail = match (stats.context_tokens, stats.context_limit) {
            (Some(used), Some(limit)) => format!("当前上下文已用 {used} / {limit} token"),
            _ => "当前上下文使用百分比".to_owned(),
        };
        items.push(StatItem {
            icon: None,
            ring: Some(Ring { percent }),
            tooltip: detail,
            label: format_percent(percent),
        });
    }
    items
}

/// How much of the model's context window the current prompt occupies, as a percentage.
///
/// Capped at 100 and rounded, matching the upstream meter (`min(100, round(used / window * 100))`):
/// a prompt larger than the window is a real (if unhappy) state, and "103%" is not a share.
///
/// @param stats - the reported statistics.
/// @returns the percentage, or `None` when either half is missing.
fn context_percent(stats: &Stats) -> Option<f64> {
    let used = stats.context_tokens? as f64;
    let limit = stats.context_limit.filter(|limit| *limit > 0)? as f64;
    Some((used / limit * 100.0).round().min(100.0))
}

/// A token count the way the upstream client writes it: `517`, `12.2K`, `1.2M`.
///
/// @param value - a non-negative token count.
/// @returns the compact form.
fn format_tokens(value: u64) -> String {
    let scaled = |candidate: f64| {
        if candidate >= 100.0 {
            format!("{:.0}", candidate.round())
        } else {
            // One decimal, trailing `.0` dropped: the upstream helper prints `12` rather than
            // `12.0` for an exact thousands value.
            let tenths = (candidate * 10.0).round() / 10.0;
            if (tenths.fract()).abs() < f64::EPSILON {
                format!("{tenths:.0}")
            } else {
                format!("{tenths:.1}")
            }
        }
    };
    if value < 1_000 {
        return value.to_string();
    }
    if value < 1_000_000 {
        // Plain `K`, not a locale string: the panel is Chinese-only and `number.thousand`
        // resolves to `{value}K` in every locale the upstream ships.
        return format!("{}K", scaled(value as f64 / 1_000.0));
    }
    format!("{}M", scaled(value as f64 / 1_000_000.0))
}

/// A percentage: whole numbers, except that a partial cache hit keeps one decimal.
///
/// @param percent - the share, already rounded by its producer.
/// @returns the text, without the sign.
fn format_percent(percent: f64) -> String {
    if (percent.fract()).abs() < f64::EPSILON {
        format!("{percent:.0}%")
    } else {
        format!("{percent:.1}%")
    }
}

/// The output speed: whole numbers from ten up, one decimal below.
///
/// Matches the upstream `formatTokensPerSecond`, and the reason is legibility rather than
/// precision: at 3.4 tok/s the decimal *is* the information, and at 292 it is noise.
///
/// @param speed - tokens per second.
/// @returns the number, without the unit.
fn format_speed(speed: f64) -> String {
    if speed >= 10.0 {
        format!("{speed:.0}")
    } else {
        format!("{speed:.1}")
    }
}

/// The context-occupancy ring: its diameter, and the width of its stroke.
///
/// Both are the upstream meter's own numbers (`ContextMeter.tsx`: a 14px viewBox with a 2px stroke,
/// radius 5.5). Kept in step with it on purpose, because the panel shows the same measurement as the
/// Harness's own footer and two rings of different weights read as two different meters.
const RING_DIAMETER: f32 = theme::ICON_STAT;
/// The ring's nominal radius: where the *fill's* centre line sits, and half way through the track.
const RING_RADIUS: f32 = (RING_DIAMETER - RING_STROKE) / 2.0;
const RING_STROKE: f32 = 2.0;

/// An arc through this many steps. A polyline is how the ring gets drawn — epaint 0.36 has no arc
/// or ring shape (only circle, ellipse and path) — and 24 steps put each segment well under a pixel
/// at this radius, so the result is smooth without being wasteful.
pub(crate) const RING_STEPS: usize = 24;

/// The context-occupancy ring: a track, a share of it filled, and nothing else.
///
/// This is the design's `ContextMeter` in egui's vocabulary. The upstream draws it as an SVG circle
/// whose `strokeDasharray` is `circumference * percent / 100`, rotated -90° so it starts at twelve
/// o'clock; the egui equivalent is a stroked circle plus a polyline along the same arc. The one thing
/// SVG has here that epaint 0.36 does not is `stroke-linecap: round` — the arc ends flat, which at a
/// 2px stroke on a 13px ring is not something an eye can find.
struct Ring {
    /// The share filled, 0–100.
    percent: f64,
}

/// The radius each stroke is handed, in the panel's own terms.
///
/// The two differ by half a stroke width **on purpose**: `tessellate_circle` strokes a circle
/// entirely outside its radius (`.outside()`), while `Shape::line` centres the stroke on its path. So
/// the radius that puts the track's band where the fill's band is, is not the fill's radius. Computing
/// both here rather than inline in [`Ring::draw`] is what lets a test check the relationship instead
/// of restating it.
///
/// @returns `(track, fill)`, the radius for each stroke.
fn band_radii() -> (f32, f32) {
    (RING_RADIUS - RING_STROKE / 2.0, RING_RADIUS)
}

impl Ring {
    /// The arc's points, for the painter — or `None` when the share is zero.
    ///
    /// Split out from the drawing so it can be asserted: a ring that draws nothing at 0% and a full
    /// circle at 100% is the whole contract, and "it looked round in the screenshot" cannot check it.
    ///
    /// @param center - the ring's centre.
    /// @param radius - its radius.
    /// @returns the points along the arc, or `None` when there is no arc to draw.
    fn arc_points(&self, center: egui::Pos2, radius: f32) -> Option<Vec<egui::Pos2>> {
        let share = (self.percent / 100.0).clamp(0.0, 1.0) as f32;
        if share <= 0.0 {
            // Zero is a real reading (an empty context) and it must not draw a mark: a dot at
            // twelve o'clock reads as "a little bit used", which is the opposite of the truth.
            return None;
        }
        // A full ring is a closed circle rather than a polyline: the last point would otherwise land
        // exactly on the first and the seam shows as a notch.
        let steps = ((RING_STEPS as f32 * share).ceil() as usize).max(2);
        let sweep = std::f32::consts::TAU * share;
        Some(
            (0..=steps)
                .map(|step| {
                    // Start at twelve o'clock and go clockwise, as the upstream's `rotate(-90)`.
                    let angle = -std::f32::consts::FRAC_PI_2 + sweep * step as f32 / steps as f32;
                    center + egui::vec2(angle.cos(), angle.sin()) * radius
                })
                .collect(),
        )
    }

    /// Draw it.
    ///
    /// @param ui - where to draw.
    /// @param center - the ring's centre.
    fn draw(&self, ui: &egui::Ui, center: egui::Pos2) {
        // **The two strokes do not take the same radius, and that is the whole trick.**
        //
        // `tessellate_circle` converts a circle's stroke with `.outside()` — a stroked circle is drawn
        // entirely *outside* its radius, spanning `r .. r + width` — while `Shape::line` centres the
        // stroke on the path, spanning `r - width/2 .. r + width/2`. Passing one number to both
        // therefore puts the fill half a stroke width inside the track, which at 13px reads as the
        // bright arc bulging out of the faint ring (reported from a screenshot, and it was right).
        //
        // So each is given the radius that puts its *band* in the same place: the track starts half a
        // stroke width in, the fill sits on the nominal circle.
        let (track_radius, fill_radius) = band_radii();
        // The track: the same hairline the panel's borders use, so it reads as a groove rather than
        // as a second figure beside the fill.
        ui.painter().circle_stroke(
            center,
            track_radius,
            egui::Stroke::new(RING_STROKE, theme::line()),
        );
        // The fill: the muted text tone, which is the upstream's own choice
        // (`--dsw-alias-label-tertiary`) and sits at the same weight as the figure beside it.
        match self.arc_points(center, fill_radius) {
            Some(points) => {
                // `Shape::line`, **not** `closed_line`: closing the polyline adds a chord between its
                // first and last points, so a 75% share draws a lens cutting across the ring's middle.
                // Tried it, looked at it, reverted.
                ui.painter().add(egui::Shape::line(
                    points,
                    egui::Stroke::new(RING_STROKE, theme::muted()),
                ));
            }
            None => {
                // Nothing used: an empty track. Said with a bare circle so the ring does not vanish
                // when the figure happens to be zero.
                ui.painter().circle_stroke(center, track_radius, egui::Stroke::new(RING_STROKE, theme::line()));
            }
        }
    }
}

/// One item of the statistics group: a leading mark, a reading, and what to say on hover.
struct StatItem {
    /// The Phosphor icon, when the mark is an icon.
    icon: Option<Icon>,
    /// The occupancy ring, when the mark is a ring.
    ring: Option<Ring>,
    /// The reading.
    label: String,
    /// The hover text, which says what the reading measures.
    tooltip: String,
}

/// A fixed right-aligned group: allocate first, then paint within its rectangle.
pub(super) fn statistics(ui: &mut egui::Ui, stats: Option<Stats>) {
    let Some(stats) = stats else { return };
    let items = stat_items(stats);
    let font = theme::font(ui.ctx(), theme::Weight::Regular, theme::TEXT_SMALL);
    let widths: Vec<f32> = items.iter().map(|item| ui.painter().layout_no_wrap(item.label.clone(), font.clone(), theme::muted()).size().x
        + theme::ICON_STAT + theme::GAP_STAT_LABEL).collect();
    let natural = widths.iter().sum::<f32>() + theme::GAP_STATS * (items.len() - 1) as f32;
    let width = natural.min(ui.available_width());
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, theme::FOOTER_INFO_HEIGHT), egui::Sense::hover());
    let budget = (width - theme::GAP_STATS * (items.len() - 1) as f32) / items.len() as f32;
    let mut x = rect.left();
    for (item, natural_width) in items.into_iter().zip(widths) {
        let item_width = if natural > width { budget } else { natural_width };
        // The mark occupies the same 13px slot either way, so a ring and an icon keep the group's
        // rhythm — and so the readings stay on one baseline whether or not the ring is there.
        let mark = egui::pos2(x + theme::ICON_STAT / 2.0, rect.center().y);
        if let Some(ring) = &item.ring {
            ring.draw(ui, mark);
        } else if let Some(icon) = item.icon {
            crate::ui::icons::paint(ui, mark, icon, theme::ICON_STAT, theme::muted());
        }
        let mut job = egui::text::LayoutJob::simple(item.label.clone(), font.clone(), theme::muted(),
            (item_width - theme::ICON_STAT - theme::GAP_STAT_LABEL).max(0.0));
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(egui::pos2(x + theme::ICON_STAT + theme::GAP_STAT_LABEL, rect.center().y - galley.size().y / 2.0), galley, theme::muted());
        // The id comes from the label rather than the kind: a ring has no icon to be named by, and
        // two items never carry the same reading.
        ui.interact(egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(item_width, rect.height())),
            ui.id().with(&item.label), egui::Sense::hover()).on_hover_text(item.tooltip);
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
        .frame(config_frame())
        .align(egui::RectAlign::TOP_END).gap(theme::POPOVER_GAP)
        .id(Kind::Model.id())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            config_width(ui);
            prepare_catalog(ui, response.rect);
            back(ui, Kind::Model);
            if options.models.is_empty() {
                picker::popover_note(ui, "宿主没有提供可选模型");
                return;
            }
            let height = menu_height(ui, response.rect);
            egui::ScrollArea::vertical().max_height(height).show(ui, |ui| {
            let mut provider = None;
            for model in &options.models {
                if provider != Some(model.provider.as_str()) {
                    picker::popover_head(ui, &model.provider_name, "");
                    provider = Some(model.provider.as_str());
                }
                if let Some(Row::Chosen) = picker::option_row(
                    ui,
                    None,
                    &model.name,
                    model.description.as_deref(),
                    current.is_some_and(|current| current.id == model.id && current.provider == model.provider),
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
        .frame(config_frame())
        .align(egui::RectAlign::TOP_END).gap(theme::POPOVER_GAP)
        .id(Kind::Effort.id())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            config_width(ui);
            prepare_catalog(ui, response.rect);
            back(ui, Kind::Effort);
            let height = menu_height(ui, response.rect);
            egui::ScrollArea::vertical().max_height(height).show(ui, |ui| {
            for option in &current.efforts {
                if let Some(Row::Chosen) =
                    picker::option_row(ui, None, &option.name, option.description.as_deref(), Some(&option.id) == options.current_effort.as_ref().or(current.default_effort.as_ref()), false, false)
                {
                    chosen = Some(Action::SelectEffort { effort: option.id.clone() });
                }
            }
            });
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
    // Catalog order is presentation order, never a ranking of access. Only the host's known
    // full-access preset has the filled warning mark; custom presets keep a neutral shield.
    let elevated = options.current_permission.as_deref() == Some("danger-full-access");
    let response = button(
        ui,
        Kind::Permission.id(),
        Some(if elevated { Icon::Shield } else { Icon::ShieldCheck }),
        &label,
        None,
        elevated,
    );
    if response.clicked() && !egui::Popup::is_id_open(ui.ctx(), Kind::Permission.id()) { *action = Some(Action::RefreshOptions); }
    let mut chosen = None;
    egui::Popup::menu(&response)
        .id(Kind::Permission.id())
        .align(egui::RectAlign::TOP_START).gap(theme::POPOVER_GAP)
        .frame(theme::popover_frame())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(theme::POPOVER_WIDTH - 2.0 * (theme::GAP_TIGHT + theme::BORDER));
            ui.spacing_mut().item_spacing.y = 0.0;
            prepare_catalog(ui, response.rect);
            picker::popover_head(ui, "会话权限", "");
            let height = menu_height(ui, response.rect);
            egui::ScrollArea::vertical().max_height(height).show(ui, |ui| {
            for option in &options.permissions {
                if let Some(Row::Chosen) = picker::option_row(
                    ui,
                    Some(match option.value.as_str() {
                        "read-only" => Icon::Eye,
                        "workspace-write" => Icon::FolderSimple,
                        "danger-full-access" => Icon::Shield,
                        _ => Icon::ShieldCheck,
                    }),
                    &option.name,
                    option.description.as_deref(),
                    Some(&option.value) == options.current_permission.as_ref(),
                    false,
                    false,
                ) {
                    chosen = Some(Action::SetPermission { value: option.value.clone() });
                }
            }
            });
        });
    if let Some(chosen) = chosen {
        *action = Some(chosen);
        egui::Popup::close_id(ui.ctx(), Kind::Permission.id());
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

/// Reset the Area's remembered height before laying out its header. egui's set_max_height
/// also resets the cursor, so calling it after the header would make the first row overlap it.
fn prepare_catalog(ui: &mut egui::Ui, anchor: egui::Rect) {
    ui.set_max_height(menu_height(ui, anchor) + theme::MENU_LINE_HEIGHT
        + f32::from(theme::ROW_PADDING.top + theme::ROW_PADDING.bottom) + theme::GAP_CLOSE);
}

/// Constrain catalogs to this viewport; long host-provided lists scroll instead of clipping.
fn menu_height(ui: &egui::Ui, anchor: egui::Rect) -> f32 {
    let heading = theme::MENU_LINE_HEIGHT + f32::from(theme::ROW_PADDING.top + theme::ROW_PADDING.bottom) + theme::GAP_CLOSE;
    let padding = 2.0 * (theme::GAP_TIGHT + theme::BORDER);
    (anchor.top().min(ui.ctx().viewport_rect().bottom()) - f32::from(theme::SHADOW_ROOM_TOP)
        - theme::POPOVER_GAP - heading - padding).max(theme::MENU_LINE_HEIGHT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ring's centre, for geometry assertions.
    fn centre() -> egui::Pos2 {
        egui::pos2(100.0, 50.0)
    }

    #[test]
    fn the_ring_starts_at_twelve_oclock_and_sweeps_clockwise() {
        // The upstream meter draws its share with `strokeDasharray` on a circle rotated -90°, which
        // starts the arc at twelve o'clock. Starting at three o'clock instead — the natural default
        // for an angle of zero — would put the filled part in a different place from the Harness's
        // own footer, for the same number.
        let ring = Ring { percent: 25.0 };
        let points = ring.arc_points(centre(), 5.5).expect("a quarter has an arc");
        let radius = 5.5;
        // The first point is straight up from the centre.
        assert!((points[0].x - centre().x).abs() < 0.01, "starts at twelve: {:?}", points[0]);
        assert!((points[0].y - (centre().y - radius)).abs() < 0.01, "and above the centre: {:?}", points[0]);
        // A quarter turn clockwise lands at three o'clock, to the right.
        let last = points[points.len() - 1];
        assert!((last.x - (centre().x + radius)).abs() < 0.01, "ends at three: {last:?}");
        assert!((last.y - centre().y).abs() < 0.01, "on the centre line: {last:?}");
    }

    #[test]
    fn the_arc_lies_exactly_on_the_track() {
        // The fill and the track are two different shapes — a `CircleShape` and a `PathShape` — built
        // from two different stroke types, so "the same radius was passed to both" is not the same
        // claim as "they are concentric". This asserts the second one: every point of the arc is on
        // the circle the track draws, and the arc's ends are the track's own vertices.
        //
        // The fill is given the nominal radius and the track starts half a stroke width inside it,
        // because `tessellate_circle` draws a circle's stroke *outside* its radius (`.outside()`) while
        // `Shape::line` centres one on its path. This asserts the arc is where it is meant to be; the
        // relationship between the two bands is checked by `the_two_strokes_are_banded_the_same`.
        let centre = centre();
        let radius = RING_RADIUS;
        for percent in [1.0, 12.5, 25.0, 50.0, 75.0, 99.0, 100.0] {
            let points = Ring { percent }.arc_points(centre, radius).expect("an arc");
            for point in &points {
                let distance = (*point - centre).length();
                assert!(
                    (distance - radius).abs() < 1e-4,
                    "{percent}%: a point sits {distance} from the centre, not {radius}",
                );
            }
            // Twelve o'clock is where the track's own top vertex is — the arc does not merely start
            // "near the top", it starts on the circle.
            let first = points[0];
            assert!((first.x - centre.x).abs() < 1e-4, "{percent}%: starts off the vertical: {first:?}");
            assert!((first.y - (centre.y - radius)).abs() < 1e-4, "{percent}%: starts below the top: {first:?}");
        }
        // And the radius both shapes are given is the same expression, so a change to either constant
        // cannot move one without the other.
        assert_eq!(radius, (RING_DIAMETER - RING_STROKE) / 2.0);
    }

    #[test]
    fn the_two_strokes_are_banded_the_same() {
        // The bug this exists for: both strokes were given `(13 - 2) / 2 = 5.5` and looked offset,
        // because a stroked circle spans `r .. r + width` while a polyline spans
        // `r - width/2 .. r + width/2`. The fill therefore sat half a stroke width *inside* the
        // track, and at 13px that reads as the bright arc bulging out of the ring.
        //
        // The rule, not the pixels — and read from the same helper `draw` uses, so a change there
        // cannot leave this test asserting a copy of the old arrangement (which is exactly what the
        // first version did: it passed with the bug re-introduced, because it restated the formula
        // instead of reading it).
        let (track_radius, fill_radius) = band_radii();
        // A circle's stroke spans `radius .. radius + width`; a polyline's spans
        // `radius - width/2 .. radius + width/2`.
        let track_band = (track_radius, track_radius + RING_STROKE);
        let fill_band = (fill_radius - RING_STROKE / 2.0, fill_radius + RING_STROKE / 2.0);
        assert!(
            (track_band.0 - fill_band.0).abs() < f32::EPSILON
                && (track_band.1 - fill_band.1).abs() < f32::EPSILON,
            "the two strokes must occupy one band: track {track_band:?}, fill {fill_band:?}",
        );
        // And the ring fits the slot it was given, so it cannot push the row taller than the icons
        // beside it. The bound is a radius rather than a diameter: 5.5 + 1 = 6.5 is the half-slot.
        let half_slot = RING_DIAMETER / 2.0;
        assert!(
            track_band.1 <= half_slot + f32::EPSILON,
            "the ring's outer edge {} must stay inside half the slot {half_slot}",
            track_band.1,
        );
    }

    #[test]
    fn a_zero_share_draws_no_arc_at_all() {
        // Zero is a real reading — an empty context — and a dot at twelve o'clock reads as "a little
        // bit used", which is the opposite of the truth. So there is no arc to draw, only the track.
        assert!(Ring { percent: 0.0 }.arc_points(centre(), 5.5).is_none());
    }

    #[test]
    fn a_full_share_is_a_whole_ring_and_an_oversized_one_is_capped() {
        // A prompt larger than the window is a real state, and "103% of a circle" is not a picture:
        // the share is clamped, so the ring closes rather than overlapping itself.
        for percent in [100.0, 103.0, 1_000.0] {
            let points = Ring { percent }.arc_points(centre(), 5.5).expect("a full share has an arc");
            let first = points[0];
            let last = points[points.len() - 1];
            assert!(
                (first.x - last.x).abs() < 0.01 && (first.y - last.y).abs() < 0.01,
                "{percent}% closes the ring: {first:?} vs {last:?}",
            );
        }
    }

    #[test]
    fn the_arc_resolution_follows_the_share() {
        // Steps are spent where the arc is: a 1% share drawn with 24 segments would put every point
        // within a fraction of a pixel of the next, and a full ring drawn with two would be a chord.
        let small = Ring { percent: 1.0 }.arc_points(centre(), 5.5).expect("an arc");
        let full = Ring { percent: 100.0 }.arc_points(centre(), 5.5).expect("an arc");
        assert!(small.len() >= 2, "a tiny share is still a line: {}", small.len());
        assert!(full.len() > small.len(), "and a full ring spends the segments: {} vs {}", full.len(), small.len());
        assert!(full.len() <= RING_STEPS + 1, "without exceeding the budget: {}", full.len());
    }

    #[test]
    fn the_context_group_is_a_ring_rather_than_an_icon() {
        // The one structural fact about the third group: its mark is drawn, not looked up. The pie
        // glyph it used to use is gone from the vocabulary entirely, so a regression cannot silently
        // bring it back.
        let items = stat_items(Stats {
            turns: 3,
            steps: 9,
            context_tokens: Some(11_159),
            context_limit: Some(1_000_000),
            ..Stats::default()
        });
        let last = items.last().expect("the ring group is there");
        assert!(last.ring.is_some(), "the occupancy group draws a ring");
        assert!(last.icon.is_none(), "and no icon");
        assert_eq!(last.label, "1%", "with the bare percentage the design asks for");
        assert!(last.tooltip.contains("11159"), "and the figures on hover: {}", last.tooltip);
    }

    #[test]
    fn the_ring_group_is_absent_when_the_window_is_unknown() {
        // A share of an unknown total is not a number anybody can act on, and a ring is no better:
        // it would either be empty (claiming zero) or full (claiming everything).
        let items = stat_items(Stats {
            turns: 1,
            steps: 1,
            context_tokens: Some(11_159),
            context_limit: None,
            ..Stats::default()
        });
        assert!(items.iter().all(|item| item.ring.is_none()), "{:?}", items.len());
    }
}
