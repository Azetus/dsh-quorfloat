//! The two pickers in the top bar: which workspace, and which conversation.
//!
//! **The bar says where the panel is; the menus are how it gets somewhere else.** Both are
//! drawn from what the host listed, and both report the user's intent as an [`Action`] —
//! nothing here attaches, creates or pins anything itself, for the same reason nothing else
//! in `ui` touches the session.
//!
//! Three design decisions are worth stating because they are choices, not translations:
//!
//! - **Choosing a workspace chooses its newest conversation.** Our model has no "current
//!   workspace" that exists without a conversation: a workspace is where conversations live,
//!   and switching to one means switching to what is in it. A workspace with nothing in it
//!   says so and points at "new conversation" rather than silently creating one.
//! - **The pin is per row and separate from the click.** Clicking a row goes there now;
//!   pinning it means "always open this one". Collapsing the two would make every casual
//!   switch permanent.
//! - **The row for a conversation shows its title only when we have one.** The session list
//!   carries no titles, so anything but the attached conversation is identified by its
//!   workspace and its age — see `docs/prototype.md` §29.

use eframe::egui;

use crate::app::PanelState;
use crate::app::session::follow::{SessionSummary, Workspace};
use crate::ui::icons::Icon;
use crate::ui::{Action, icons, theme};

/// The popup id of one picker, for the development switch in `ui/mod.rs`.
///
/// @param kind - which picker.
/// @returns the id its open state lives under.
pub(crate) fn popup_id(kind: Kind) -> egui::Id {
    kind.id()
}

/// Which picker a popup belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// The workspace picker.
    Workspace,
    /// The conversation picker.
    Conversation,
}

impl Kind {
    /// The id the popup's open state is remembered under.
    pub(super) fn id(self) -> egui::Id {
        egui::Id::new(("quorfloat-picker", self as u8))
    }
}

/// What a row reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    /// The user chose this one.
    Chosen,
    /// The user pinned it, or unpinned it.
    PinToggled,
}

/// What a picker did, and where its button is.
///
/// The rect travels back because the top bar is the window's drag handle, and a handle that
/// covers a button is a button that cannot be pressed: the bar subtracts what its controls
/// occupy and leaves the gaps draggable (`ui/mod.rs`).
#[derive(Debug, Clone)]
pub(super) struct Outcome {
    /// What the user asked for, if anything.
    pub action: Option<Action>,
    /// The button's rectangle.
    pub button: egui::Rect,
}

/// The workspace picker.
///
/// @param ui - where to draw.
/// @param state - everything the panel knows, including the workspace list.
/// @returns what the user asked for, and where the button is.
pub(super) fn workspaces(ui: &mut egui::Ui, state: &PanelState) -> Outcome {
    let current = state
        .current_workspace
        .as_deref()
        .or(state.pinned_workspace.as_deref())
        .and_then(|id| state.workspaces.iter().find(|workspace| workspace.workspace_id == id))
        .map_or_else(|| "工作区".to_owned(), |workspace| workspace.title.clone());
    let button = picker_button(ui, Kind::Workspace, Icon::Folder, &current, state.pinned_workspace.is_some());

    let mut action = None;
    egui::Popup::menu(&button)
        .id(Kind::Workspace.id())
        .gap(theme::GAP_TIGHT)
        .frame(theme::popover_frame())
        // A row's pin button must not dismiss the menu, so only clicks outside it close.
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(theme::POPOVER_WIDTH);
            popover_head(ui, "工作区", &format!("{} 个", state.workspaces.len()));
            if state.workspaces.is_empty() {
                // Asked for once, when the menu is first opened: a request per frame would be
                // a panel that spends its life asking the same question. After that the list is
                // whatever the host had, and the note below says what to do about it.
                if !state.workspaces_asked {
                    popover_note(ui, "正在向 Harness 查询工作区…");
                    action = Some(Action::RefreshWorkspaces);
                } else {
                    popover_note(ui, "Harness 里还没有工作区。");
                    popover_hint(ui, "在 Harness 中打开一个文件夹，再回来点「重新查询」。");
                }
                if let Some(Row::Chosen) = option_row(ui, Icon::Search, "重新查询", None, false, false, false) {
                    action = Some(Action::RefreshWorkspaces);
                }
            }
            for workspace in &state.workspaces {
                let chosen = state.current_workspace.as_deref() == Some(workspace.workspace_id.as_str());
                let pinned = state.pinned_workspace.as_deref() == Some(workspace.workspace_id.as_str());
                let detail = newest_in(state, workspace).map(|newest| short_time(newest.updated_at, now()));
                match option_row(ui, Icon::Folder, &workspace.title, detail.as_deref(), chosen, pinned, true) {
                    Some(Row::Chosen) => {
                        // Switching workspace means switching to what is in it. With nothing
                        // in it there is nowhere to go, and the hint below says so.
                        if let Some(newest) = newest_in(state, workspace) {
                            action = Some(Action::ChooseConversation { session_id: newest.session_id.clone() });
                            egui::Popup::close_id(ui.ctx(), Kind::Workspace.id());
                        }
                    }
                    Some(Row::PinToggled) => {
                        action = Some(Action::PinWorkspace {
                            workspace_id: (!pinned).then(|| workspace.workspace_id.clone()),
                        });
                    }
                    None => {}
                }
            }
            separator(ui);
            popover_hint(ui, "固定在某个工作区后，「新建会话」会在那里创建。");
        });
    Outcome { action, button: button.rect }
}

/// The conversation picker.
///
/// @param ui - where to draw.
/// @param state - everything the panel knows, including the conversation list.
/// @returns what the user asked for, and where the button is.
pub(super) fn conversations(ui: &mut egui::Ui, state: &PanelState) -> Outcome {
    // A title beats an id wherever we have one: the harness names a conversation once it has
    // something to name it after, and `6e089e47` is what the panel shows when it has nothing
    // better — see `conversation_name` for why only the attached one has a title at all.
    let current = state
        .attached
        .as_deref()
        .map(|id| {
            state
                .title
                .as_deref()
                .filter(|title| !title.trim().is_empty())
                .map_or_else(|| short_id(id).to_owned(), str::to_owned)
        })
        .unwrap_or_else(|| "新会话".to_owned());
    let button = picker_button(ui, Kind::Conversation, Icon::Chat, &current, state.pinned.is_some());

    let mut action = None;
    egui::Popup::menu(&button)
        .id(Kind::Conversation.id())
        .gap(theme::GAP_TIGHT)
        .frame(theme::popover_frame())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(theme::POPOVER_WIDTH);
            // The header says *where* these conversations are, not how many there are: the
            // workspace is what tells two similarly named conversations apart, and the count is
            // something the list shows by existing.
            popover_head(ui, "会话", &workspace_title(state));
            if let Some(Row::Chosen) =
                option_row(ui, Icon::Plus, "开始新会话", Some("发送时创建"), false, false, false)
            {
                // Not "create one now": the design creates the conversation when the user
                // submits, so this clears whatever the panel was pinned or switched to and
                // leaves it waiting for something to say.
                action = Some(Action::NewConversation);
                egui::Popup::close_id(ui.ctx(), Kind::Conversation.id());
            }
            if state.conversations.is_empty() {
                popover_note(ui, "Harness 里还没有会话。");
            }
            // Conversations nobody has said anything in are left out: the harness keeps a record for
            // every session that was opened and abandoned, and a list of untitled, empty entries is a
            // list the user has to read to find the one they meant. The exception is the conversation the
            // panel is in — a session created by sending the first message is blank for exactly one
            // moment, and hiding the row the panel is attached to would be worse than a spare row.
            let attached = state.attached.as_deref();
            let pinned = state.pinned.as_deref();
            for conversation in state.conversations.iter().filter(|conversation| {
                !conversation.blank
                    || attached == Some(conversation.session_id.as_str())
                    || pinned == Some(conversation.session_id.as_str())
            }) {
                let attached = state.attached.as_deref() == Some(conversation.session_id.as_str());
                let pinned = state.pinned.as_deref() == Some(conversation.session_id.as_str());
                let name = conversation_name(state, conversation);
                let detail = describe(conversation, now());
                match option_row(ui, Icon::Chat, &name, Some(&detail), attached, pinned, true) {
                    Some(Row::Chosen) => {
                        action = Some(Action::ChooseConversation { session_id: conversation.session_id.clone() });
                        egui::Popup::close_id(ui.ctx(), Kind::Conversation.id());
                    }
                    Some(Row::PinToggled) => {
                        action = Some(Action::PinConversation {
                            session_id: (!pinned).then(|| conversation.session_id.clone()),
                        });
                    }
                    None => {}
                }
            }
            separator(ui);
            popover_hint(ui, "固定会话后，每次呼出继续此会话。");
        });
    Outcome { action, button: button.rect }
}

/// The most recent conversation in a workspace, if it has one.
fn newest_in<'a>(state: &'a PanelState, workspace: &Workspace) -> Option<&'a SessionSummary> {
    state
        .conversations
        .iter()
        .filter(|conversation| conversation.cwd.as_deref().is_some_and(|cwd| same_directory(&workspace.path, cwd)))
        .max_by_key(|conversation| conversation.updated_at)
}

/// Whether two directories are the same one.
///
/// @param left - one path, as the host wrote it.
/// @param right - the other.
/// @returns whether they name the same directory.
#[must_use]
pub fn same_directory(left: &str, right: &str) -> bool {
    let trim = |path: &str| path.trim_end_matches(['/', '\\']).to_owned();
    !left.is_empty() && trim(left) == trim(right)
}

/// What to call a conversation in a list.
///
/// The attached one has a title, because its `session/title` event has arrived; the others
/// are identified by the shortest thing that is still unique — see the note at the top.
fn conversation_name(state: &PanelState, conversation: &SessionSummary) -> String {
    // The name the Harness gave it, from whichever side knows: the attached conversation's own
    // `session/title` event (which is fresher — a title can be rewritten as the conversation
    // develops), and otherwise the title the host carried in the list.
    if state.attached.as_deref() == Some(conversation.session_id.as_str()) {
        if let Some(title) = state.title.as_deref().filter(|title| !title.trim().is_empty()) {
            return title.to_owned();
        }
    }
    if let Some(title) = conversation.title.as_deref().filter(|title| !title.trim().is_empty()) {
        return title.to_owned();
    }
    short_id(&conversation.session_id).to_owned()
}

/// A conversation's second line: how old it is, and what it is doing.
///
/// The workspace used to lead this line; the design puts it once in the header, and repeating it
/// on every row is what makes a menu taller than the thing it lists.
fn describe(conversation: &SessionSummary, now: i64) -> String {
    let mut parts: Vec<String> = vec![short_time(conversation.updated_at, now)];
    if conversation.running {
        parts.push("生成中".to_owned());
    } else if conversation.blank {
        parts.push("空白".to_owned());
    }
    parts.join(" · ")
}

/// The name of the workspace the panel is in, for the conversation menu's header.
fn workspace_title(state: &PanelState) -> String {
    state
        .current_workspace
        .as_deref()
        .or(state.pinned_workspace.as_deref())
        .and_then(|id| state.workspaces.iter().find(|workspace| workspace.workspace_id == id))
        .map_or_else(String::new, |workspace| workspace.title.clone())
}

/// The shortest unique-looking form of a session id.
///
/// The host's ids look like `session-6e089e47-ba4a-…`; the first block after the prefix is
/// what the panel already shows elsewhere, and what a person can compare by eye.
///
/// @param session_id - the durable identity.
/// @returns the short form.
#[must_use]
pub fn short_id(session_id: &str) -> &str {
    // Only the host's own shape is shortened. Anything else is shown whole, because
    // truncating an id we do not recognise at its first hyphen could turn two different
    // conversations into the same three letters.
    let Some(rest) = session_id.strip_prefix("session-") else {
        return session_id;
    };
    match rest.split('-').next() {
        Some(first) if !first.is_empty() => first,
        _ => session_id,
    }
}

/// How long ago something happened, in the fewest words that are still true.
///
/// @param updated_at - when it happened, in epoch milliseconds.
/// @param now - now, in epoch milliseconds.
/// @returns a phrase such as `刚刚` or `3 小时前`.
#[must_use]
pub fn short_time(updated_at: i64, now: i64) -> String {
    let seconds = (now - updated_at) / 1000;
    match seconds {
        // Clock skew between the host and this process is real, and "in the future" is not a
        // thing a session list should ever say.
        ..=59 => "刚刚".to_owned(),
        60..=3_599 => format!("{} 分钟前", seconds / 60),
        3_600..=86_399 => format!("{} 小时前", seconds / 3_600),
        // "Yesterday" rather than "1 天前", because that is how the design words the same
        // band — and it is computed from elapsed time rather than from a calendar, because
        // this process has no timezone to compare days in. `今天 14:32` would need one; see
        // `docs/prototype.md` §34 for why that is a decision rather than an oversight.
        86_400..=172_799 => "昨天".to_owned(),
        _ => format!("{} 天前", seconds / 86_400),
    }
}

/// Now, in epoch milliseconds, for the age columns.
fn now() -> i64 {
    crate::ipc::rpc::now_millis()
}

/// The button a popover hangs from.
///
/// @param ui - where to draw.
/// @param kind - which picker, for the id that remembers whether it is open.
/// @param icon - the leading mark.
/// @param label - what it says.
/// @param marked - whether something about it is pinned, which is drawn in the accent colour.
/// @returns the response, for the popup to anchor to.
fn picker_button(ui: &mut egui::Ui, kind: Kind, icon: Icon, label: &str, marked: bool) -> egui::Response {
    let open = egui::Popup::is_id_open(ui.ctx(), kind.id());
    // Measured, because the button has to make room for its label and then clip it: the
    // tools beside it stay where they are whatever a workspace or a conversation is called.
    let label_width = ui
        .painter()
        .layout(
            label.to_owned(),
            theme::font(ui.ctx(), theme::Weight::Regular, theme::TEXT_META),
            theme::text(),
            f32::INFINITY,
        )
        .size()
        .x;
    let width = (label_width + theme::ICON_PICKER * 2.0 + theme::ICON_CHEVRON + theme::GAP_TIGHT * 3.0 + 14.0)
        .min(220.0);
    let height = theme::ICON + 10.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    if open || response.hovered() {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(theme::RADIUS_PICKER), theme::soft());
    }
    let mut cursor = rect.left() + 7.0;
    let middle = rect.center().y;
    icons::paint(ui, egui::pos2(cursor + theme::ICON_PICKER / 2.0, middle), icon, theme::ICON_PICKER, theme::muted());
    cursor += theme::ICON_PICKER + theme::GAP_TIGHT;
    // The label is clipped rather than allowed to push the tools off the bar; egui has no
    // ellipsis on a painter, so the truncation happens through the layout width above.
    ui.painter().text(
        egui::pos2(cursor, middle),
        egui::Align2::LEFT_CENTER,
        label,
        theme::font(ui.ctx(), theme::Weight::Regular, theme::TEXT_META),
        theme::text(),
    );
    let colour = if marked { theme::accent() } else { theme::muted() };
    icons::paint(
        ui,
        egui::pos2(rect.right() - 7.0 - theme::ICON_CHEVRON / 2.0, middle),
        Icon::CaretDown,
        theme::ICON_CHEVRON,
        colour,
    );
    response
}

/// The title line of a popover: what the list is, and how much of it there is.
fn popover_head(ui: &mut egui::Ui, title: &str, detail: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).size(theme::TEXT_SMALL).color(theme::text()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(detail).size(theme::TEXT_SMALL).color(theme::muted()));
        });
    });
    ui.add_space(2.0);
}

/// A popover's closing advice.
fn popover_hint(ui: &mut egui::Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(egui::RichText::new(text).size(theme::TEXT_SMALL).color(theme::muted()));
}

/// One line of nothing much, for a list that is still empty.
fn popover_note(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(theme::TEXT_SMALL).color(theme::muted()));
}

/// The line between the list and the hint.
fn separator(ui: &mut egui::Ui) {
    ui.add_space(4.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), theme::BORDER), egui::Sense::hover());
    ui.painter().rect_filled(rect, egui::CornerRadius::ZERO, theme::line());
    ui.add_space(4.0);
}

/// One choosable row: an icon, a name, a second line, and a pin.
///
/// @param ui - where to draw.
/// @param icon - the leading mark.
/// @param name - the row's main text.
/// @param detail - the second line, when there is one.
/// @param chosen - whether this is the one in effect, which draws a tick.
/// @param pinned - whether this one is pinned, which fills the pin.
/// @param pinnable - whether this row offers a pin at all.
/// @returns what the row reported, if anything.
fn option_row(
    ui: &mut egui::Ui,
    icon: Icon,
    name: &str,
    detail: Option<&str>,
    chosen: bool,
    pinned: bool,
    pinnable: bool,
) -> Option<Row> {
    let mut reported = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let pin_width = if pinnable { theme::ICON_BUTTON } else { 0.0 };
        let width = ui.available_width() - pin_width;
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(width, theme::TEXT_META * 2.0 + 6.0),
            egui::Sense::click(),
        );
        if response.hovered() {
            ui.painter().rect_filled(rect, egui::CornerRadius::same(theme::RADIUS_PICKER), theme::soft());
        }
        let mut cursor = rect.left() + 9.0;
        let middle = rect.center().y;
        icons::paint(ui, egui::pos2(cursor + theme::ICON_PICKER / 2.0, middle), icon, theme::ICON_PICKER, theme::muted());
        cursor += theme::ICON_PICKER + theme::GAP_TIGHT;
        ui.painter().text(
            egui::pos2(cursor, middle - 8.0),
            egui::Align2::LEFT_CENTER,
            name,
            theme::font(ui.ctx(), theme::Weight::Regular, theme::TEXT_META),
            theme::text(),
        );
        if let Some(detail) = detail {
            ui.painter().text(
                egui::pos2(cursor, middle + 8.0),
                egui::Align2::LEFT_CENTER,
                detail,
                theme::font(ui.ctx(), theme::Weight::Regular, theme::TEXT_SMALL),
                theme::muted(),
            );
        }
        if chosen {
            icons::paint(
                ui,
                egui::pos2(rect.right() - 9.0 - theme::ICON_PICKER / 2.0, middle),
                Icon::Check,
                theme::ICON_PICKER,
                theme::accent(),
            );
        }
        if response.clicked() {
            reported = Some(Row::Chosen);
        }

        if pinnable {
            let (pin_rect, pin) = ui.allocate_exact_size(
                egui::vec2(theme::ICON_BUTTON, theme::ICON_BUTTON),
                egui::Sense::click(),
            );
            if pin.hovered() || pinned {
                let fill = if pinned { theme::soft() } else { theme::soft() };
                ui.painter().rect_filled(pin_rect, egui::CornerRadius::same(theme::RADIUS_SMALL_BUTTON), fill);
            }
            let colour = if pinned { theme::accent() } else { theme::muted() };
            let mark = if pinned { Icon::PushPin } else { Icon::PushPinSlash };
            icons::paint(ui, pin_rect.center(), mark, theme::ICON_PICKER, colour);
            if pin.clicked() {
                reported = Some(Row::PinToggled);
            }
            if pin.hovered() {
                pin.on_hover_text(if pinned { "取消固定" } else { "固定" });
            }
        }
    });
    reported
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_id_is_shown_as_its_shortest_distinguishing_part() {
        assert_eq!(short_id("session-6e089e47-ba4a-4695-a368-2a0a5a1411a4"), "6e089e47");
        // An id the host wrote some other way is shown as it is rather than mangled.
        assert_eq!(short_id("plain-id"), "plain-id");
        assert_eq!(short_id("session-"), "session-");
    }

    #[test]
    fn age_is_said_in_the_fewest_words_that_are_still_true() {
        let now = 1_000_000_000;
        assert_eq!(short_time(now, now), "刚刚");
        assert_eq!(short_time(now - 59_000, now), "刚刚");
        assert_eq!(short_time(now - 60_000, now), "1 分钟前");
        assert_eq!(short_time(now - 3_599_000, now), "59 分钟前");
        assert_eq!(short_time(now - 3_600_000, now), "1 小时前");
        assert_eq!(short_time(now - 86_400_000, now), "昨天");
        assert_eq!(short_time(now - 172_799_000, now), "昨天");
        assert_eq!(short_time(now - 172_800_000, now), "2 天前");
        assert_eq!(short_time(now - 86_400_000 * 9, now), "9 天前");
        // A clock that disagrees with the host's must not produce "in the future".
        assert_eq!(short_time(now + 60_000, now), "刚刚");
    }

    #[test]
    fn two_paths_are_the_same_directory_or_they_are_not() {
        assert!(same_directory("/work/project", "/work/project"));
        assert!(same_directory("/work/project/", "/work/project"));
        assert!(same_directory("C:\\\\work\\\\project", "C:\\\\work\\\\project"));
        // A subdirectory is a different directory: labelling a session opened in `src/` with
        // the workspace it happens to sit under would be a guess, and a wrong one as soon as
        // two workspaces are nested.
        assert!(!same_directory("/work/project", "/work/project/src"));
        assert!(!same_directory("", ""));
        assert!(!same_directory("/work/a", "/work/b"));
    }
}
