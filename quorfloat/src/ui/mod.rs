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
mod picker;
mod settings;
mod table;

/// The one thing the app needs from the picker: the id of the workspace menu, so a send that
/// has no workspace to create in can open the menu that chooses one.
pub(crate) use picker::{Kind as PickerKind, popup_id as picker_popup_id};
/// The display-only soft wrapper, reachable from the panel's tests.
#[cfg(test)]
pub(crate) use conversation::soft_wrap_for_display;
pub mod fonts;
pub mod icons;
pub mod screenshot;
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
    /// Attach to one conversation, now.
    ChooseConversation {
        /// Which one.
        session_id: String,
    },
    /// Go back to "new conversation" mode, without creating anything yet.
    ///
    /// The creation itself happens when the user submits their first message: a panel that
    /// created a conversation per summon would fill the harness's list with empty ones.
    NewConversation,
    /// Keep opening one conversation, or stop pinning any.
    PinConversation {
        /// Which one, or `None` to unpin.
        session_id: Option<String>,
    },
    /// Ask for the workspace list, which the picker needs the first time it opens.
    RefreshWorkspaces,
    /// Pin the workspace new conversations are created in, or stop pinning one.
    PinWorkspace {
        /// Which one, or `None` to unpin.
        workspace_id: Option<String>,
    },
    /// Show the settings view, replacing the conversation.
    OpenSettings,
    /// Leave the settings view and go back to the conversation.
    CloseSettings,
    /// Draw the panel in one of the design's two palettes, or follow the platform.
    SetTheme(theme::Preference),
    /// Keep the panel open when the user moves to another window, or let it put itself away.
    KeepOpenOnBlur(bool),
}

/// What the drawing layer learned about the panel's own size.
///
/// The window follows the conversation: compact while there is nothing to read, growing as
/// an answer arrives, and capped so that a long one scrolls instead of covering the screen.
/// That makes the height a *result* of drawing, which is why it comes back from here rather
/// than being computed by the caller — only this layer knows how tall the content turned out
/// to be.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PanelLayout {
    /// How tall the panel would like to be, in logical pixels.
    pub desired_height: f32,
    /// Everything above the conversation: the top bar and the composer.
    pub chrome_above: f32,
    /// The conversation's own padding, top and bottom.
    pub thread_padding: f32,
    /// How tall the conversation's content turned out to be.
    pub thread_content: f32,
    /// The footer's predicted height.
    pub footer: f32,
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
/// @returns how tall the panel wants to be, for the window to follow.
pub(crate) fn draw(
    ui: &mut egui::Ui,
    state: &PanelState,
    draft: &mut String,
    action: &mut Option<Action>,
    markdown: &mut egui_commonmark::CommonMarkCache,
) -> PanelLayout {
    open_picker_from_env(ui.ctx());
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
                    let panel_top = ui.min_rect().top();
                    top_bar(ui, state, action);
                    // The settings page **replaces** the conversation rather than covering it, which
                    // is what the design does and the only arrangement that fits a panel this size:
                    // a dialog would have to be smaller than the thing it hides. The top bar stays,
                    // so the user can still see which conversation they are about to go back to.
                    let settings_view = state.settings_open;
                    if settings_view {
                        let (outcome, back) = settings::settings(ui, state);
                        if back {
                            *action = Some(Action::CloseSettings);
                        }
                        if let Some(chosen) = outcome.action() {
                            *action = Some(chosen);
                        }
                    } else {
                        composer(ui, state, draft, action);
                        if let Some(handoff) = &state.handoff {
                            handoff_banner(ui, handoff, action);
                        }
                        cards(ui, state, action);
                    }
                    // Everything above the conversation, measured rather than predicted:
                    // this is the distance from the panel's top edge to where the thread
                    // starts, and it is what makes "how tall does the panel want to be" a
                    // question with an answer instead of an estimate. In the settings view there is
                    // no thread, so the same measurement is what the page itself occupies.
                    let chrome_above = ui.cursor().min.y - panel_top;
                    let footer = footer_height(ui);
                    if settings_view {
                        footer_bar(ui, state);
                        return PanelLayout {
                            desired_height: (chrome_above + footer)
                                .clamp(MIN_PANEL_HEIGHT, state.max_height),
                            chrome_above,
                            thread_padding: 0.0,
                            thread_content: 0.0,
                            footer,
                        };
                    }
                    // The composer claimed its share by being drawn first; the footer is
                    // below the conversation and has to be predicted, or a long
                    // conversation pushes the panel's own hints off the bottom. The thread's
                    // own padding is part of that arithmetic — forgetting it is how the
                    // first version drew prose against the panel's border.
                    // The rule above the thread is drawn inside this frame too, so its one pixel
                    // is part of the height being counted.
                    let thread_padding = f32::from(theme::PAD_THREAD.top + theme::PAD_THREAD.bottom)
                        + theme::BORDER;
                    let thread = (ui.available_height() - footer - thread_padding).max(0.0);
                    let content = egui::Frame::NONE
                        .inner_margin(theme::PAD_THREAD)
                        .show(ui, |ui| {
                            // The line the design draws above the thread: it separates the
                            // conversation from the composer without a heading.
                            // One hairline, and nothing else: the frame's own inner margin is
                            // the space above the thread, and adding to it here is how the panel
                            // came out 20px taller than the height it had counted (§30, and the
                            // invariant test that caught it again).
                            rule(ui);
                            conversation(ui, state, thread, markdown)
                        })
                        .inner;
                    footer_bar(ui, state);
                    PanelLayout {
                        desired_height: (chrome_above + thread_padding + content + footer)
                            .clamp(MIN_PANEL_HEIGHT, state.max_height),
                        chrome_above,
                        thread_padding,
                        thread_content: content,
                        footer,
                    }
                })
                .inner
        })
        .inner
}

/// Open one picker at startup, when the environment asks for it.
///
/// A development aid with no part in normal running: menus open on a click, and a screenshot
/// cannot click. `DSH_QUORFLOAT_OPEN_PICKER=session` or `=workspace` makes the panel come up
/// with that menu already open, which is how its layout is checked by eye.
///
/// @param ctx - the context whose memory holds the popup state.
fn open_picker_from_env(ctx: &egui::Context) {
    let Ok(which) = std::env::var("DSH_QUORFLOAT_OPEN_PICKER") else {
        return;
    };
    let id = match which.trim().to_ascii_lowercase().as_str() {
        "session" => picker::popup_id(picker::Kind::Conversation),
        "workspace" => picker::popup_id(picker::Kind::Workspace),
        _ => return,
    };
    if !egui::Popup::is_id_open(ctx, id) {
        egui::Popup::open_id(ctx, id);
    }
}

/// Whether the environment asks for the settings view to come up open.
///
/// The same kind of development aid as `open_picker_from_env`, and for the same reason: a
/// screenshot cannot click a button, and the settings page is the one view a run without a session
/// cannot otherwise reach. `DSH_QUORFLOAT_OPEN_SETTINGS=1` asks for it.
///
/// @returns whether the panel should start on the settings page.
#[must_use]
pub fn settings_requested() -> bool {
    std::env::var("DSH_QUORFLOAT_OPEN_SETTINGS")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !value.is_empty() && value != "0" && value != "false"
        })
        .unwrap_or(false)
}

/// The shortest the panel is allowed to be.
///
/// The top bar, the composer and the footer, with room to see that the conversation is
/// empty: below this the panel would hide the controls it exists to offer.
pub(super) const MIN_PANEL_HEIGHT: f32 = 168.0;

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
                        egui::RichText::new("DeepSeek")
                            .font(theme::font(ui.ctx(), theme::Weight::Medium, theme::TEXT_BRAND))
                            .extra_letter_spacing(theme::BRAND_LETTER_SPACING)
                            .color(text()),
                    );
                    ui.add_space(theme::GAP);
                    // Where the panel is: the workspace, then the conversation inside it. Both
                    // are pickers now — the conversation list exists, so the carets open
                    // something (see `ui/picker.rs`).
                    let mut occupied = Vec::new();
                    let workspace = picker::workspaces(ui, state);
                    if workspace.action.is_some() {
                        *action = workspace.action.clone();
                    }
                    occupied.push(workspace.button);
                    ui.label(egui::RichText::new("/").size(theme::TEXT_META).color(theme::line()));
                    let conversations = picker::conversations(ui, state);
                    if conversations.action.is_some() {
                        *action = conversations.action.clone();
                    }
                    occupied.push(conversations.button);
                    // The tools, pushed to the far end.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // The panel's own top-right corner: the two buttons are placed against it
                        // rather than against the cursor, so they stay put when the pickers to
                        // their left change width.
                        let corner = egui::pos2(ui.max_rect().right(), ui.max_rect().top());
                        let hint = match &state.hotkey {
                            Some(spec) => format!("收起面板（{spec}）"),
                            None => "收起面板".to_owned(),
                        };
                        let close = corner_icon_button(
                            ui,
                            icons::Icon::Close,
                            &hint,
                            false,
                            corner,
                        );
                        occupied.push(close.rect);
                        if close.clicked() || ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                            *action = Some(Action::Hide);
                        }
                        // The settings, next to the way out: both are about the panel rather than
                        // about the conversation, and the design puts them together at this end.
                        // It stays lit while the page is open, which is how the user can tell that
                        // they are on it — the design's `aria-pressed`.
                        let gear = corner_icon_button(
                            ui,
                            icons::Icon::GearSix,
                            "悬浮窗设置",
                            state.settings_open,
                            corner,
                        );
                        occupied.push(gear.rect);
                        if gear.clicked() {
                            *action = Some(if state.settings_open {
                                Action::CloseSettings
                            } else {
                                Action::OpenSettings
                            });
                        }
                    });
                    occupied
                })
                // The occupied rects travel out through the frames that drew them, because
                // the drag handle is computed from them below.
                .inner
            })
            .inner
        });
    let occupied = bar.inner;
    // The bar is the window's drag handle — an undecorated window has no title bar — but it
    // is also where the controls live, and a handle registered over a button swallows the
    // press that button was waiting for. So the handle is what the controls leave behind:
    // drag anywhere that is not a control.
    for (index, region) in drag_regions(bar.response.rect, &occupied).into_iter().enumerate() {
        let handle = ui.interact(region, ui.id().with(("quorfloat-drag", index)), egui::Sense::drag());
        if handle.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if handle.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
    }
}

/// The narrowest gap worth dragging by.
///
/// A two-pixel slot between two buttons is not a target; it is a way to move the window by
/// accident while aiming at something else.
const MIN_DRAG_WIDTH: f32 = 10.0;

/// The parts of a bar that are left over once its controls have taken their share.
///
/// Pure, so that the interesting cases — controls that touch, a bar with nothing left, a
/// layout whose gaps are too narrow to use — can be checked without a window.
///
/// @param bar - the whole bar.
/// @param occupied - what the controls occupy, in any order.
/// @returns the draggable regions, left to right.
#[must_use]
fn drag_regions(bar: egui::Rect, occupied: &[egui::Rect]) -> Vec<egui::Rect> {
    let mut spans: Vec<(f32, f32)> = occupied.iter().map(|rect| (rect.left(), rect.right())).collect();
    spans.sort_by(|left, right| left.0.total_cmp(&right.0));

    let mut regions = Vec::new();
    let mut cursor = bar.left();
    for (left, right) in spans {
        // Only the part of a control that is inside the bar can take space away from it.
        let left = left.clamp(bar.left(), bar.right());
        let right = right.clamp(bar.left(), bar.right());
        if left > cursor {
            regions.push(egui::Rect::from_min_max(
                egui::pos2(cursor, bar.top()),
                egui::pos2(left, bar.bottom()),
            ));
        }
        cursor = cursor.max(right);
    }
    if cursor < bar.right() {
        regions.push(egui::Rect::from_min_max(
            egui::pos2(cursor, bar.top()),
            egui::pos2(bar.right(), bar.bottom()),
        ));
    }
    regions.retain(|region| region.width() >= MIN_DRAG_WIDTH);
    regions
}

/// A square icon button, at the design's size for the top bar.
///
/// @param ui - where to draw.
/// @param icon - which picture.
/// @param tooltip - what it does, shown on hover.
/// @returns the response, so the caller can act on a click.
pub(super) fn icon_button(
    ui: &mut egui::Ui,
    icon: icons::Icon,
    tooltip: &str,
) -> egui::Response {
    let size = egui::vec2(theme::ICON_BUTTON, theme::ICON_BUTTON);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    paint_icon_button(ui, rect, icon, tooltip, false, &response)
}

/// The same button, placed against a known corner and able to stay lit.
///
/// The top bar's two controls sit at the panel's own top-right corner, and the design's rectangle
/// for them is that corner inset by the bar's padding. Measuring from the corner rather than from
/// wherever the layout cursor happens to be is what makes the button's position a fact a test can
/// compute — and what keeps it in place when the pickers to its left change width, which is the
/// reason the row is laid out right-to-left at all.
///
/// @param ui - where to draw.
/// @param icon - which picture.
/// @param tooltip - what it does, shown on hover.
/// @param lit - whether to report "this control's view is open" by staying highlighted.
/// @param corner - the panel's own top-right corner.
/// @returns the response, so the caller can act on a click.
pub(super) fn corner_icon_button(
    ui: &mut egui::Ui,
    icon: icons::Icon,
    tooltip: &str,
    lit: bool,
    corner: egui::Pos2,
) -> egui::Response {
    let size = egui::vec2(theme::ICON_BUTTON, theme::ICON_BUTTON);
    let rect = egui::Rect::from_min_size(
        egui::pos2(
            corner.x - f32::from(theme::PAD_TOP.right) - size.x,
            corner.y + f32::from(theme::PAD_TOP.top),
        ),
        size,
    );
    let response =
        ui.interact(rect, ui.id().with((icon.name(), "top-bar")), egui::Sense::click());
    paint_icon_button(ui, rect, icon, tooltip, lit, &response)
}

/// What both of the above draw: the surface, the glyph, and the hint.
///
/// @param ui - where to draw.
/// @param rect - the button's rectangle.
/// @param icon - which picture.
/// @param tooltip - what it does, shown on hover.
/// @param lit - whether the control's view is open.
/// @param response - the interaction already registered for `rect`.
/// @returns the response, for the caller to read `clicked` from.
fn paint_icon_button(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    icon: icons::Icon,
    tooltip: &str,
    lit: bool,
    response: &egui::Response,
) -> egui::Response {
    // A button whose view is open keeps its surface, which is the design's `aria-pressed`: the top
    // bar should never leave the user guessing which control put them where they are.
    if lit || response.hovered() {
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(theme::RADIUS_ICON_BUTTON),
            theme::soft(),
        );
    }
    let colour = if lit { theme::accent() } else { ui.style().interact(response).fg_stroke.color };
    // The icon family, not the text one: a private-use codepoint laid out in a text font is
    // a tofu box, which is exactly what the first look at this panel showed.
    icons::paint(ui, rect.center(), icon, theme::ICON, colour);
    response.clone().on_hover_text(tooltip)
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
    // The section's own padding: every other section has one, and this one went without, which
    // is why the card's box sat closer to the panel's edge than the composer's content did.
    egui::Frame::NONE
        .inner_margin(theme::PAD_CARDS)
        .show(ui, |ui| {
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
        });
}

/// The height the footer will occupy, before it is drawn.
///
/// Predicted from the design's own padding and type, for the same reason the composer's
/// height is: the conversation is given what is left, and a first frame that guessed would
/// make the panel jump on the second. It is one line, and that is a promise the footer has
/// to keep — the first version drew four lines into a prediction of one, and the last two
/// were clipped off the bottom of the panel.
///
/// @param ui - the frame, for the item spacing.
/// @returns the strip's height.
fn footer_height(ui: &egui::Ui) -> f32 {
    theme::TEXT_SMALL + 4.0 + f32::from(theme::PAD_FOOTER.top + theme::PAD_FOOTER.bottom) + ui.spacing().item_spacing.y
}

/// Which turn the panel is on, in the design's wording.
fn turn_label(state: &PanelState) -> String {
    let turns = state
        .entries
        .iter()
        .filter(|entry| matches!(entry, crate::app::session::transcript::Entry::User { .. }))
        .count();
    format!("第 {turns} 轮")
}

/// A hairline across the panel, the design's separator between sections.
fn rule(ui: &mut egui::Ui) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, theme::BORDER), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0, theme::line());
}

/// One keyboard hint: a chip with the key on it, and what it does.
///
/// @param ui - where to draw.
/// @param keys - the glyphs to draw inside the chip, left to right.
/// @param what - what the key does.
fn kbd(ui: &mut egui::Ui, keys: &[icons::Icon], what: &str) {
    egui::Frame::NONE
        .fill(theme::soft())
        .stroke(egui::Stroke::new(theme::BORDER, theme::line()))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS_KBD))
        .inner_margin(egui::Margin { left: 4, right: 4, top: 1, bottom: 1 })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                for key in keys {
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(theme::TEXT_SMALL, theme::TEXT_SMALL),
                        egui::Sense::hover(),
                    );
                    icons::paint(ui, rect.center(), *key, theme::TEXT_SMALL, muted());
                }
                if keys.is_empty() {
                    ui.label(
                        egui::RichText::new("esc").size(theme::TEXT_SMALL).color(muted()),
                    );
                }
            });
        });
    ui.label(theme::meta(ui.ctx(), what));
}

/// The bottom bar: what the keys do, and what the panel is doing.
///
/// One row, always: the shortcut hints on the left, and whatever the panel has to report on
/// the right. The keys are spelled out rather than drawn as `↵` and `⇧` because the bundled
/// fonts have no glyphs for them — the first version showed two tofu boxes where the user
/// was supposed to read a keyboard.
fn footer_bar(ui: &mut egui::Ui, state: &PanelState) {
    let frame = egui::Frame::NONE.inner_margin(theme::PAD_FOOTER);
    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            // While a turn is being worked on the hints give way to what is happening: the design does
            // the same (its `shortcuts()` is restored when the turn ends), and a key hint is worth
            // less than knowing whether the model is still going.
            if state.turn_active {
                ui.label(theme::meta(
                    ui.ctx(),
                    &format!("{} · 正在生成", turn_label(state)),
                ));
                return;
            }
            ui.spacing_mut().item_spacing.x = theme::GAP_TIGHT;
            // The keys are drawn as chips, which is how the design shows them: a key is a thing
            // you press, and a bordered box says so at a glance. `KEY_RETURN` and a fat up arrow
            // are the font's nearest glyphs to ↵ and ⇧ — the shift symbol it does not have.
            kbd(ui, &[icons::Icon::KeyReturn], "发送");
            kbd(ui, &[icons::Icon::ShiftUp, icons::Icon::KeyReturn], "换行");
            kbd(ui, &[], "关闭");
            // The right-hand side is empty unless there is something to say. It used to report
            // what the panel was following on every frame, which under the current design is
            // both stale ("following the newest" no longer exists) and noise: the session picker
            // in the top bar answers that question, and the composer answers the one that
            // matters — whether a message can be sent.
            let status = state
                .prompt_line
                .clone()
                .or_else(|| state.fonts_warning.clone());
            if let Some(status) = status {
                let colour = if state.prompt_line.is_none() && state.fonts_warning.is_some() {
                    warn_text()
                } else {
                    muted()
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(status).size(theme::TEXT_SMALL).color(colour),
                        )
                        .truncate(),
                    );
                });
            }
        });
    });
}

#[cfg(test)]
mod drag_tests {
    use super::*;

    /// A bar 400 wide, 40 tall, at the top of the panel.
    fn bar() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 40.0))
    }

    fn rect(left: f32, right: f32) -> egui::Rect {
        egui::Rect::from_min_max(egui::pos2(left, 0.0), egui::pos2(right, 40.0))
    }

    #[test]
    fn the_gaps_between_controls_are_what_can_be_dragged() {
        // Two controls in the middle: the space before, between and after them.
        let regions = drag_regions(bar(), &[rect(100.0, 150.0), rect(200.0, 250.0)]);
        let spans: Vec<(f32, f32)> = regions.iter().map(|region| (region.left(), region.right())).collect();
        assert_eq!(spans, vec![(0.0, 100.0), (150.0, 200.0), (250.0, 400.0)]);
    }

    #[test]
    fn a_slot_too_narrow_to_aim_at_is_not_a_handle() {
        // Eight pixels between two buttons is a way to move the window while aiming at one of
        // them, not a target.
        let regions = drag_regions(bar(), &[rect(100.0, 200.0), rect(208.0, 300.0)]);
        let spans: Vec<(f32, f32)> = regions.iter().map(|region| (region.left(), region.right())).collect();
        assert_eq!(spans, vec![(0.0, 100.0), (300.0, 400.0)]);
    }

    #[test]
    fn a_bar_that_is_all_controls_has_nothing_left_to_drag_by() {
        // The panel can still be moved by the hotkey and by the conversation area's own
        // scrolling; what must not happen is a handle sitting on a button.
        assert!(drag_regions(bar(), &[rect(0.0, 400.0)]).is_empty());
        assert!(drag_regions(bar(), &[rect(-50.0, 450.0)]).is_empty());
        assert!(drag_regions(bar(), &[]).len() == 1, "an empty bar is entirely draggable");
    }

    #[test]
    fn controls_outside_the_bar_do_not_eat_into_it() {
        // A popup's rect or a mis-measured one must not turn the bar's own space into a gap.
        let regions = drag_regions(bar(), &[rect(-100.0, -10.0), rect(500.0, 600.0)]);
        assert_eq!(regions.len(), 1);
        assert_eq!((regions[0].left(), regions[0].right()), (0.0, 400.0));
    }
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
