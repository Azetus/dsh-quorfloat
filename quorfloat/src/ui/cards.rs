//! The cards: what the host is asking, and what the user may do about it.
//!
//! Above the conversation because they are the only part of the panel with a deadline:
//! an approval is waiting for an answer, and a card that has to be scrolled to is a card
//! that gets missed.

use eframe::egui;

use crate::app::session::interaction::{ApprovalVerdict, Handoff, Interaction, InteractionKind, InteractionState};

use super::Action;
use crate::ui::theme as theme;
use crate::ui::theme::DISPLAY_LOCALE;

use super::wrapped;

/// Draw a label that wraps, so peer-supplied text cannot widen the panel.
///
/// @param ui - where to draw.
/// @param text - the already-styled text.

/// Draw the announcement that a request was routed to another surface.
///
/// The wording has to be true, and "true" depends on what the host reported. A hint
/// now means "the panel did not take this" — the host only announces a hand-off when
/// it defers — so the panel must not imply it is holding the request. And "go to the
/// Harness window" is only advice it can give when a surface is actually live: with
/// none, the honest sentence is that nothing can answer it.
///
/// @param ui - where to draw.
/// @param handoff - what the host announced.
/// @param action - collects the dismissal if the user asks for one.
pub(super) fn handoff_banner(ui: &mut egui::Ui, handoff: &Handoff, action: &mut Option<Action>) {
    let live = !handoff.surfaces().is_empty();
    let (headline, instruction) = match (handoff.kind(), live) {
        (InteractionKind::Question, true) => ("追问待回答", "请切到 Harness 窗口输入"),
        (InteractionKind::Question, false) => ("追问待回答", "当前没有可以回答它的 Harness 窗口"),
        (InteractionKind::Approval, true) => ("审批待确认", "请切到 Harness 窗口确认"),
        (InteractionKind::Approval, false) => ("审批待确认", "当前没有可以确认它的 Harness 窗口"),
    };
    egui::Frame::NONE
        .fill(theme::banner())
        .inner_margin(egui::Margin::same(10))
        .corner_radius(6)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(headline).size(13.0).color(theme::warn_text()));
                ui.label(egui::RichText::new(instruction).size(12.0).color(theme::text()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("知道了").clicked() {
                        *action = Some(Action::DismissHandoff);
                    }
                });
            });
        });
}

/// Draw one approval card, with its buttons only while an answer is still possible.
///
/// @param ui - where to draw.
/// @param card - the interaction to render.
/// @param action - collects what the user clicked.
pub(super) fn approval_card(ui: &mut egui::Ui, card: &Interaction, action: &mut Option<Action>) {
    theme::card_frame().show(ui, |ui| {
            let headline = match card.tool_name() {
                Some(tool) => format!("工具 {tool} 请求提权"),
                None => "有工具请求提权".to_owned(),
            };
            ui.label(theme::card_title(ui.ctx(), headline));
            ui.add_space(theme::GAP_CLOSE);
            wrapped(
                ui,
                egui::RichText::new(
                    card.detail(DISPLAY_LOCALE).unwrap_or_else(|| "请求方没有说明原因".to_owned()),
                )
                .font(theme::font(ui.ctx(), theme::Weight::Regular, theme::TEXT_META))
                .color(theme::muted()),
            );
            ui.add_space(theme::GAP_CARD);
            match card.state() {
                InteractionState::Pending => {
                    ui.horizontal(|ui| {
                        // One filled action, one surface: the design draws exactly one filled
                        // control perview, and a green "allow" beside a red "reject" is two.
                        // The verdict's colour still appears — as the *status* text once the
                        // answer lands, which is where the design uses those colours too.
                        if ui.add(theme::primary_button(ui.ctx(), "允许一次")).clicked() {
                            *action = Some(Action::Answer {
                                id: card.id().to_owned(),
                                verdict: ApprovalVerdict::AllowOnce,
                            });
                        }
                        if ui.add(theme::secondary_button(ui.ctx(), "拒绝")).clicked() {
                            *action = Some(Action::Answer {
                                id: card.id().to_owned(),
                                verdict: ApprovalVerdict::Reject,
                            });
                        }
                    });
                }
                InteractionState::Submitting { .. } => {
                    // Buttons are gone rather than disabled: a second click would be a
                    // second decision for one question, and the host refuses it anyway.
                    ui.label(theme::meta(ui.ctx(), "已提交，等待 Harness 确认…"));
                }
                InteractionState::Applied { verdict } => {
                    let (text, colour) = match verdict {
                        ApprovalVerdict::AllowOnce => ("已允许一次", theme::good_text()),
                        ApprovalVerdict::Reject => ("已拒绝", theme::bad_text()),
                    };
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(text)
                                .font(theme::font(ui.ctx(), theme::Weight::Regular, theme::TEXT_META))
                                .color(colour),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            close_button(ui, card, action);
                        });
                    });
                }
                InteractionState::Refused { reason } => {
                    wrapped(
                        ui,
                        egui::RichText::new(format!("已失效：{reason}"))
                            .font(theme::font(ui.ctx(), theme::Weight::Regular, theme::TEXT_META))
                            .color(theme::bad_text()),
                    );
                    ui.add_space(theme::GAP_CLOSE);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close_button(ui, card, action);
                    });
                }
            }
        });
}

/// Draw one question card.
///
/// It has no answer widgets, and says so. This build cannot answer a question — the
/// answer shape is a list of selected option ids, and there is no widget for that
/// yet — and a card that offered a disabled button would imply the panel could
/// answer if only the user tried harder. What it *can* do is show the question, which
/// is what lets the user decide whether to switch to the Harness window.
///
/// @param ui - where to draw.
/// @param card - the interaction to render.
/// @param action - collects what the user clicked.
pub(super) fn question_card(ui: &mut egui::Ui, card: &Interaction, action: &mut Option<Action>) {
    let questions = card.questions();
    theme::card_frame().show(ui, |ui| {
            ui.label(theme::card_title(
                ui.ctx(),
                &format!("追问（{} 个问题）", card.question_count()),
            ));
            ui.add_space(theme::GAP_CLOSE);
            wrapped(
                ui,
                egui::RichText::new("本窗口不能回答追问，请切到 Harness 窗口输入")
                    .font(theme::font(ui.ctx(), theme::Weight::Regular, theme::TEXT_META))
                    .color(theme::warn_text()),
            );
            for (header, question) in &questions {
                ui.add_space(theme::GAP_TIGHT);
                // The choices are drawn in the picker row's own shape, because that is where the
                // user meets the same kind of choice — the card is read-only, but the options
                // must not look like a bulleted list of prose.
                theme::option_frame().show(ui, |ui| {
                    let text = match header {
                        Some(header) => format!("{header} — {question}"),
                        None => question.clone(),
                    };
                    wrapped(
                        ui,
                        egui::RichText::new(text)
                            .font(theme::font(ui.ctx(), theme::Weight::Regular, theme::TEXT_BODY))
                            .color(theme::text()),
                    );
                });
            }
            if card.question_count() > questions.len() {
                ui.add_space(theme::GAP_TIGHT);
                wrapped(ui, theme::meta(ui.ctx(), "（还有更多，未全部显示）"));
            }
            ui.add_space(theme::GAP_CARD);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(theme::primary_button(ui.ctx(), "知道了")).clicked() {
                    *action = Some(Action::Dismiss { id: card.id().to_owned() });
                }
            });
        });
}

/// Draw the button that stops showing a resolved card.
///
/// @param ui - where to draw.
/// @param card - the card being closed.
/// @param action - collects the dismissal.
pub(super) fn close_button(ui: &mut egui::Ui, card: &Interaction, action: &mut Option<Action>) {
    // The same ⨯ as the top bar's: closing a card and closing the panel are the same gesture,
    // and a text button here would be the third kind of button in one panel.
    if super::icon_button(ui, super::icons::Icon::Close, "关闭这张卡片").clicked() {
        *action = Some(Action::Dismiss { id: card.id().to_owned() });
    }
}
