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
mod footer;
mod conversation;
mod geometry;
mod picker;
mod settings;
mod table;

/// The one thing the app needs from the picker: the id of the workspace menu, so a send that
/// has no workspace to create in can open the menu that chooses one.
pub(crate) use footer::{Kind as FooterKind, popup_id as footer_popup_id};
/// The editor's own single-line height, reachable from the panel's tests so that the composer's layout
/// is asserted against this number rather than against a copy of it.
#[cfg(test)]
pub(crate) use composer::EDITOR_MIN_HEIGHT;
#[cfg(test)]
pub(crate) use footer::{fill_radius_for_test, track_radius_for_test};
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
    /// Listen for a chord and use it as the accelerator.
    StartHotkeyRecording,
    /// Stop listening, leaving the registration alone.
    StopHotkeyRecording,
    /// Refuse the chord that was just pressed, and say why in the row.
    RejectHotkey {
        /// What is wrong with it, phrased for the user.
        hint: String,
    },
    /// Switch the conversation's model, and the reasoning effort to pair with it.
    ///
    /// Both together, because the host holds them in one selection.
    SelectModel {
        /// Provider route of the chosen model.
        provider: String,
        /// The chosen model's id.
        model: String,
        /// The effort to pair with it, or `None` for the host's default.
        effort: Option<String>,
    },
    /// Change only the reasoning effort, keeping the current model.
    SelectEffort {
        /// The chosen effort's id.
        effort: String,
    },
    /// Apply a permission preset to the conversation.
    SetPermission {
        /// The preset's stable value.
        value: String,
    },
    /// Ask the host what may be chosen, after a read failed or before the first one.
    RefreshOptions,
    /// Hold a different global accelerator.
    ///
    /// The accelerator in the host's spelling — the same string the host writes in its own config and
    /// the same one `hello` reports — because that spelling is the value the settings page shows.
    SetHotkey(String),
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
            let far_shadow = ui.painter().add(egui::Shape::Noop);
            let panel = egui::Frame::NONE
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
                    if state.settings_open {
                        let top = ui.cursor().min.y - panel_top;
                        // The session's strip is not part of this page, so it is neither drawn nor
                        // reserved for — the page gets that height back.
                        let footer = footer_height(ui, state, Page::Settings);
                        let spacing = ui.spacing().item_spacing.y;
                        let border = theme::BORDER * 2.0;
                        let available = (ui.available_height() - footer - spacing)
                            .min(state.max_height - border - top - footer - spacing)
                            .max(0.0);
                        let (outcome, back, content) = settings::settings(ui, state, available);
                        if back {
                            *action = Some(Action::CloseSettings);
                        }
                        if let Some(chosen) = outcome.action() {
                            *action = Some(chosen);
                        }
                        footer_bar(ui, state, action, Page::Settings);
                        let chrome_above = top + content + spacing;
                        return PanelLayout {
                            desired_height: (border + chrome_above + footer)
                                .clamp(MIN_PANEL_HEIGHT, state.max_height),
                            chrome_above,
                            thread_padding: 0.0,
                            thread_content: 0.0,
                            footer,
                        };
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
                    // question with an answer instead of an estimate.
                    let chrome_above = ui.cursor().min.y - panel_top;
                    let footer = footer_height(ui, state, Page::Conversation);
                    if state.entries.is_empty() && state.live.is_none() {
                        footer_bar(ui, state, action, Page::Conversation);
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
                    // is part of the height being counted, and so is the space the design puts
                    // under it (see below).
                    let thread_padding = f32::from(theme::PAD_THREAD.bottom) + theme::BORDER
                        + f32::from(theme::PAD_THREAD.top);
                    let thread = (ui.available_height() - footer - thread_padding).max(0.0);
                    let content = egui::Frame::NONE
                        .inner_margin(egui::Margin { top: 0, ..theme::PAD_THREAD })
                        .show(ui, |ui| {
                            // **The hairline is the thread's own top border**, which is how the design
                            // draws it: `.q-thread { border-top:1px solid var(--q-line); padding:20px 24px
                            // 22px }` — the padding is *under* the line. Drawing it above the line instead
                            // left the composer's reserved strip in place (the space the design keeps for
                            // its clipboard row, which this panel does not have): 34px of empty panel
                            // between the input and the line that is supposed to divide it from the thread.
                            //
                            // The 20px is spent here rather than inside the scrolling thread, so the first
                            // line never ends up pressed against the line while the user reads upwards.
                            rule(ui);
                            ui.add_space(f32::from(theme::PAD_THREAD.top));
                            conversation(ui, state, thread, markdown)
                        })
                        .inner;
                    footer_bar(ui, state, action, Page::Conversation);
                    PanelLayout {
                        desired_height: (chrome_above + thread_padding + content + footer)
                            .clamp(MIN_PANEL_HEIGHT, state.max_height),
                        chrome_above,
                        thread_padding,
                        thread_content: content,
                        footer,
                    }
                });
            // The window can still be shrinking toward its content. Both shadows must follow the
            // actual rounded panel, not the old viewport's available rectangle.
            ui.painter().set(far_shadow, theme::shadow_far().as_shape(
                panel.response.rect,
                egui::CornerRadius::same(theme::RADIUS_WINDOW),
            ));
            panel.inner
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
        // The footer's three, which need the panel to have been given options by the host: a
        // screenshot cannot click a button, and the arrangement is the thing being looked at.
        "config" => footer_popup_id(FooterKind::Config),
        "model" => footer_popup_id(FooterKind::Model),
        "effort" => footer_popup_id(FooterKind::Effort),
        "permission" => footer_popup_id(FooterKind::Permission),
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

/// What the composer should start with, when the environment names a draft.
///
/// The same kind of development aid as [`settings_requested`], and here for a layout that cannot be
/// photographed otherwise: the composer grows with explicit lines, the send button is pinned to the
/// top of the row, and **a screenshot cannot type**. `DSH_QUORFLOAT_DRAFT=$'first\nsecond'` puts those
/// two lines in the box so the grown row can be looked at.
///
/// Unset or empty means an empty composer, exactly as before.
///
/// @returns the text to put in the composer.
#[must_use]
pub fn draft_from_env() -> String {
    std::env::var("DSH_QUORFLOAT_DRAFT").unwrap_or_default()
}

/// Whether the environment asks for the hotkey row to come up listening.
///
/// The same kind of development aid as [`settings_requested`], for the same reason: the recording state
/// draws differently from the chip, and a screenshot cannot click the box that enters it.
/// `DSH_QUORFLOAT_RECORD_HOTKEY=1` asks for it.
///
/// @returns whether the row should start recording.
#[must_use]
pub fn hotkey_recording_requested() -> bool {
    std::env::var("DSH_QUORFLOAT_RECORD_HOTKEY")
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
    let bar = egui::Frame::NONE.inner_margin(theme::PAD_TOP).show(ui, |ui| {
        ui.style_mut().interaction.selectable_labels = false;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), theme::ICON_BUTTON), egui::Sense::hover());
        let close_rect = egui::Rect::from_min_size(
            egui::pos2(rect.right() - theme::ICON_BUTTON, rect.top()), egui::Vec2::splat(theme::ICON_BUTTON));
        let gear_rect = close_rect.translate(egui::vec2(-theme::ICON_BUTTON - theme::GAP_CLOSE, 0.0));
        let content = egui::Rect::from_min_max(rect.min, egui::pos2(gear_rect.left() - theme::GAP_TOP, rect.bottom()));
        let mut occupied = vec![close_rect, gear_rect];
        ui.scope_builder(egui::UiBuilder::new().max_rect(content).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = theme::GAP_CLOSE;
            ui.label(egui::RichText::new("DeepSeek")
                .font(theme::font(ui.ctx(), theme::Weight::Medium, theme::TEXT_BRAND))
                .extra_letter_spacing(theme::BRAND_LETTER_SPACING).color(text()));
            ui.add_space(theme::GAP_TOP - theme::GAP_CLOSE);
            let room = ui.available_rect_before_wrap();
            let workspace_room = egui::Rect::from_min_size(room.min, egui::vec2((room.width() - theme::GAP_TOP) / 2.0, rect.height()));
            let workspace = ui.scope_builder(egui::UiBuilder::new().max_rect(workspace_room)
                .layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| picker::workspaces(ui, state)).inner;
            if workspace.action.is_some() { *action = workspace.action; }
            occupied.push(workspace.button);
            ui.label(egui::RichText::new("/").size(theme::TEXT_META).color(theme::line()));
            let conversation = picker::conversations(ui, state);
            if conversation.action.is_some() { *action = conversation.action; }
            occupied.push(conversation.button);
        });
        let hint = state.hotkey.as_ref().map_or_else(|| "收起面板".to_owned(), |spec| format!("收起面板（{spec}）"));
        let close = ui.interact(close_rect, ui.id().with("close"), egui::Sense::click());
        paint_icon_button(ui, close_rect, icons::Icon::Close, &hint, false, &close);
        if close.clicked() || (!state.recording && ui.input(|input| input.key_pressed(egui::Key::Escape))) {
            *action = Some(Action::Hide);
        }
        let gear = ui.interact(gear_rect, ui.id().with("settings"), egui::Sense::click());
        paint_icon_button(ui, gear_rect, icons::Icon::GearSix, "悬浮窗设置", state.settings_open, &gear);
        if gear.clicked() { *action = Some(if state.settings_open { Action::CloseSettings } else { Action::OpenSettings }); }
        occupied
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
    let colour = if lit { theme::accent() } else { theme::muted() };
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

/// Which page a bottom bar is measured and drawn for.
///
/// **The session's own controls belong to the conversation page**: its permission, its model and
/// effort, its statistics. They sit in a strip of their own, a *sibling* of the scrolling thread and
/// pinned under it, and they go away with that page when the settings replace it — in web-UI terms the
/// strip lives at the bottom of the conversation's own `div`, not beside it. What both pages keep is
/// the panel's own row: the key hints, and whatever the panel has to say (`已提交，等待 Harness
/// 确认…`, a refused setting, a missing font).
///
/// The design puts the strip *outside* `q-main` (as a sibling of `q-main` and `q-settings`), which is
/// why it used to stay on the settings page; that is what the user asked to change, and why this
/// distinction exists at all (see `docs/dsh-quorfloat.md` §11.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    /// The conversation: a thread, and the session's controls under it.
    Conversation,
    /// The settings page, which is about the panel: the session's controls are not part of it.
    Settings,
}

/// Reserve the exact two strips and their separator before sizing the conversation.
fn runtime_height(state: &PanelState) -> f32 {
    if state.options.as_ref().is_some_and(|options| !options.models.is_empty() || !options.permissions.is_empty()) {
        theme::FOOTER_PICKER_HEIGHT + f32::from(theme::PAD_RUNTIME.bottom)
    } else { 0.0 }
}

/// Total reserved height, including the layout's trailing item spacing.
///
/// The session's strip is counted only on the page that draws it: reserving height for a strip that is
/// not there is how a panel comes out taller than its own content.
fn footer_height(ui: &egui::Ui, state: &PanelState, page: Page) -> f32 {
    let session = if page == Page::Conversation { runtime_height(state) } else { 0.0 };
    session
        + theme::BORDER + theme::FOOTER_INFO_HEIGHT
        + f32::from(theme::PAD_FOOTER.top + theme::PAD_FOOTER.bottom)
        + ui.spacing().item_spacing.y
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
fn kbd(ui: &mut egui::Ui, key: &str, what: &str) {
    // The key is **text, not an icon**, because that is what the design draws: its own markup is
    // `<kbd>↵</kbd> 发送　<kbd>⇧ ↵</kbd> 换行　<kbd>esc</kbd> 关闭` — the symbols are the font's
    // own characters inside a bordered box. Drawing an icon here instead is what made the chips
    // disagree with the design.
    //
    // One character of the design's set could not be kept: **`↵` (U+21B5) has no glyph in the
    // bundled fonts** and would render as a tofu box, so Return is `⏎` (U+23CE) — the same idea,
    // and present. `⇧` is present as drawn. Both were checked by asking the font
    // (`fonts::has_glyph`), because "it looks right in the design file" says nothing about our
    // fonts; a test pins it.
    egui::Frame::NONE
        .fill(theme::soft())
        .stroke(egui::Stroke::new(theme::BORDER, theme::line()))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS_KBD))
        .inner_margin(egui::Margin { left: 4, right: 4, top: 1, bottom: 1 })
        .show(ui, |ui| {
            ui.label(egui::RichText::new(key).size(theme::TEXT_SMALL).color(muted()));
        });
    ui.label(egui::RichText::new(what).size(theme::TEXT_SMALL).color(muted()));
}

/// The footer's key hints: the chip's key, then what it does.
///
/// **The one place they are written.** The drawing reads this list and the glyph test reads this
/// list, so a test can never be checking a copy that has drifted from what is on screen — which is
/// exactly how a missing glyph (`↵`) gets shipped.
pub(crate) const KEY_HINTS: &[(&str, &str)] = &[("⏎", "发送"), ("⇧ ⏎", "换行"), ("esc", "关闭")];

/// Every character the footer's key chips contain, for the font to be asked about.
///
/// Test-only: the drawing reads [`KEY_HINTS`] directly, so this exists purely so the glyph test can
/// ask about the same strings that are drawn rather than a copy of them.
///
/// @returns the symbols and letters that must have glyphs.
#[cfg(test)]
pub(crate) fn key_symbols() -> String {
    KEY_HINTS.iter().map(|(key, _)| *key).collect()
}


/// The session's strip above, the panel's own row below, each in explicitly allocated rectangles.
///
/// @param page - which page this bar belongs to; the session's strip is drawn on the conversation page
///   only, and its statistics give way to the panel's own line (`status_line`) when it has one.
fn footer_bar(ui: &mut egui::Ui, state: &PanelState, action: &mut Option<Action>, page: Page) {
    let width = ui.available_width();
    let height = footer_height(ui, state, page) - ui.spacing().item_spacing.y;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let session = page == Page::Conversation && runtime_height(state) > 0.0;
    let runtime = egui::Rect::from_min_max(
        rect.min + egui::vec2(f32::from(theme::PAD_RUNTIME.left), 0.0),
        egui::pos2(rect.right() - f32::from(theme::PAD_RUNTIME.right), rect.top() + theme::FOOTER_PICKER_HEIGHT),
    );
    if session {
        ui.scope_builder(egui::UiBuilder::new().max_rect(runtime), |ui| {
            ui.set_width(runtime.width());
            footer::settings(ui, state, action);
        });
    }
    // The separator belongs to the strip it closes: the session strip when there is one, and the
    // content above the bar otherwise (the settings rows, on that page).
    let rule_y = rect.top() + if session { runtime_height(state) } else { 0.0 };
    ui.painter().hline(rect.x_range(), rule_y, egui::Stroke::new(theme::BORDER, theme::line()));
    let info = egui::Rect::from_min_max(
        egui::pos2(rect.left() + f32::from(theme::PAD_FOOTER.left), rule_y + theme::BORDER + f32::from(theme::PAD_FOOTER.top)),
        rect.max - egui::vec2(f32::from(theme::PAD_FOOTER.right), f32::from(theme::PAD_FOOTER.bottom)),
    );
    let left = egui::Rect::from_min_max(info.min, egui::pos2(info.left() + theme::FOOTER_HINT_WIDTH, info.bottom()));
    ui.scope_builder(egui::UiBuilder::new().max_rect(left).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = theme::GAP_TIGHT;
        if state.turn_active {
            ui.add(egui::Label::new(theme::meta(ui.ctx(), &format!("{} · 正在生成", turn_label(state)))).truncate());
        } else {
            // The order the design uses: the primary action first.
            for (key, what) in KEY_HINTS {
                kbd(ui, key, what);
            }
        }
    });
    let right = egui::Rect::from_min_max(egui::pos2(left.right() + theme::GAP, info.top()), info.max);
    ui.scope_builder(egui::UiBuilder::new().max_rect(right).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
        if let Some(status) = status_line(state) {
            ui.add(egui::Label::new(egui::RichText::new(status.text).size(theme::TEXT_SMALL).color(status.colour)).truncate());
        } else if page == Page::Conversation {
            // Session statistics are the conversation's, so they are the session strip's counterpart:
            // the settings page shows nothing here unless the panel has something to say.
            footer::statistics(ui, state.stats);
        }
    });
}

/// What the panel has to report, and how loudly.
struct Status {
    /// The sentence.
    text: String,
    /// Its colour.
    colour: egui::Color32,
}

/// The one line the panel owes the user, if any.
///
/// Precedence is by how much the user needs to know: what just happened to their message, then why a
/// setting did not change, then the font warning that has been true since startup.
///
/// @param state - everything the panel knows.
/// @returns the line and its colour, or `None` when there is nothing to say.
fn status_line(state: &PanelState) -> Option<Status> {
    if let Some(prompt) = &state.prompt_line {
        return Some(Status { text: prompt.clone(), colour: muted() });
    }
    if let Some(failure) = &state.setting_failure {
        return Some(Status { text: failure.clone(), colour: warn_text() });
    }
    state
        .fonts_warning
        .as_ref()
        .map(|warning| Status { text: warning.clone(), colour: warn_text() })
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
