//! The bridge between the frontend and the session.
//!
//! One snapshot shape ([`snapshot`]), and the commands the page can issue. The
//! snapshot is a projection — the frontend receives what it draws, never the
//! session's own types — and it is rebuilt from the same state the session tests
//! already guard, so the bridge adds no business rules of its own: every command
//! here is a thin call into [`crate::app::session::Session`], and everything the
//! snapshot carries was decided there.
//!
//! The one rule that lives here rather than in the session is the shell's own:
//! which preferences the settings page may set, and which accelerators it may
//! hold ([`apply_preferences`] / [`validate_hotkey`]).

use serde_json::{json, Value};

use crate::app::height::Height;
use crate::app::workspace;
use crate::app::preferences::Preferences;
use crate::app::session::interaction::{
    ApprovalVerdict, Interaction, InteractionState, Question, QuestionAnswer, SentAnswer,
};
use crate::app::session::transcript::{Block, Entry};
use crate::app::session::{Delivery, FrameSink, Session, SessionOptions, Stats};
use crate::app::window_settings::WindowSettings;

/// The locale the panel asks the asker's own text for.
///
/// Every string this file writes is Chinese; the interaction detail is requested
/// in Chinese too and falls back to `en` (see `Interaction::detail`).
const DISPLAY_LOCALE: &str = "zh";

/// The shell-level facts the snapshot carries beside the session.
///
/// Kept in one place so the snapshot is a pure function of two inputs: the
/// session (owned by the reader) and this view (owned by the shell).
#[derive(Debug, Clone)]
pub struct ShellView {
    /// The effective window configuration.
    pub settings: WindowSettings,
    /// The user's own choices, which sit on top of the host's configuration.
    ///
    /// Kept here rather than in a second place so the overlay is applied once,
    /// in `with_preferences`, wherever the baseline changes.
    pub preferences: Preferences,
    /// The accelerator the user asked for, as the settings chip shows it.
    pub hotkey_requested: Option<String>,
    /// The accelerator actually held, which after a refusal is not the requested one.
    pub hotkey_held: Option<String>,
    /// Whether a grab is held right now.
    pub hotkey_registered: bool,
    /// Why the last attempt did not become the held one, when it did not.
    pub hotkey_reason: Option<String>,
    /// The workspace a *new* conversation defaults to (the P0 rung), when there
    /// is no session pin. With a session pin, this is its projection — the
    /// pinned conversation's own workspace — and the frontend hides the pin
    /// control for it.
    pub pinned_workspace: Option<String>,
    /// Whether the panel's native window is on screen right now.
    ///
    /// Owned by the dispatcher — it is the only thread that touches the window —
    /// and carried here so the snapshot can tell the frontend which way the
    /// transition should run.
    pub visible: bool,
    /// The height coordination state machine.
    pub height: Height,
}

/// The state the frontend renders, as JSON.
///
/// @param session - the session, which owns every business fact.
/// @param shell - the shell's own facts (settings, hotkey, height).
/// @returns the snapshot the frontend re-renders from.
#[must_use]
pub fn snapshot(session: &Session, shell: &ShellView) -> Value {
    let follow = session.follow();
    let transcript = session.transcript();
    let settings = &shell.settings;
    json!({
        "settings": {
            "width": settings.width,
            "maxHeight": settings.max_height,
            "alwaysOnTop": settings.always_on_top,
            "reduceMotion": settings.reduce_motion,
            "hideOnBlur": settings.hide_on_blur,
            "theme": theme_name(settings.theme),
            // The panel's language, in the Harness's own ids: the frontend picks its copy
            // from it and the menu bar (a Rust-side surface) is labelled from the same
            // value. Always present and always one of `zh` / `en` — the host resolves the
            // setting and the first-launch seed before any frame reaches this process.
            "language": settings.language.as_str(),
        },
        "hotkey": {
            "requested": shell.hotkey_requested,
            "held": shell.hotkey_held,
            "registered": shell.hotkey_registered,
            "reason": shell.hotkey_reason,
        },
        "height": {
            "target": shell.height.target(),
            "capped": shell.height.is_capped(),
        },
        "window": {
            "visible": shell.visible,
        },
        "session": {
            "ready": session.is_ready(),
            "following": follow.session_id().map(|session_id| json!({
                "sessionId": session_id,
                "label": follow.label(),
                "generation": follow.generation(),
                "events": follow.events(),
                "streams": follow.streams(),
                "resyncs": follow.resyncs(),
                "stale": follow.stale(),
            })),
            "pinned": session.pinned_conversation(),
            "pinnedWorkspace": shell.pinned_workspace,
            // Where a conversation submitted right now would be created. The panel shows it in
            // its workspace field, so that state and the ladder cannot disagree: without it the
            // field says "选择工作区" while the create is already resolved to a real one. It is
            // the same resolution `submit` uses, with no panel choice to prefer — the panel's own
            // pick is not the shell's to know.
            "createWorkspace": create_workspace(session, None, shell.pinned_workspace.as_deref()),
            "chosen": session.target_conversation(),
            "createFailure": follow.create_failure(),
            "conversations": session
                .conversations()
                .iter()
                .map(|summary| json!({
                    "sessionId": summary.session_id,
                    "updatedAt": summary.updated_at,
                    "title": summary.title,
                    "label": summary.label,
                    "cwd": summary.cwd,
                    "running": summary.running,
                    "blank": summary.blank,
                }))
                .collect::<Vec<_>>(),
            "workspaces": session
                .workspaces()
                .iter()
                .map(|workspace| json!({
                    "workspaceId": workspace.workspace_id,
                    "title": workspace.title,
                    "path": workspace.path,
                }))
                .collect::<Vec<_>>(),
            "workspacesAsked": session.workspaces_asked(),
            "options": session.options().map(options_view),
            "stats": session.stats().map(|stats| stats_view(&stats)),
            "settingFailure": session.setting_failure(),
        },
        "transcript": {
            "sessionId": transcript.session_id(),
            "cursor": transcript.cursor(),
            "title": transcript.title(),
            "streaming": transcript.is_streaming(),
            "turnActive": transcript.is_turn_active(),
            "entries": transcript.entries().iter().map(entry_view).collect::<Vec<_>>(),
            "live": transcript.live_entry().as_ref().map(entry_view),
        },
        "interactions": session.interactions().iter().map(interaction_view).collect::<Vec<_>>(),
        "handoff": session.handoff().map(|handoff| json!({
            "kind": handoff.kind().as_str(),
            "reason": handoff.reason(),
            "surfaces": handoff.surfaces(),
        })),
        "delivery": {
            "prompt": delivery_view(session.prompt_delivery()),
            "cancel": delivery_view(session.cancel_delivery()),
        },
    })
}

/// One transcript line, in the frontend's vocabulary.
fn entry_view(entry: &Entry) -> Value {
    match entry {
        Entry::User { text } => json!({ "kind": "user", "text": text }),
        Entry::Assistant { blocks, streaming } => json!({
            "kind": "assistant",
            "streaming": streaming,
            "blocks": blocks.iter().map(block_view).collect::<Vec<_>>(),
        }),
        Entry::Tool { name, text, is_error } => {
            json!({ "kind": "tool", "name": name, "text": text, "isError": is_error })
        }
        Entry::System { text } => json!({ "kind": "system", "text": text }),
    }
}

/// One content block, in the frontend's vocabulary.
fn block_view(block: &Block) -> Value {
    match block {
        Block::Text(text) => json!({ "kind": "text", "text": text }),
        Block::Reasoning(text) => json!({ "kind": "reasoning", "text": text }),
        Block::Call { name, arguments } => json!({ "kind": "call", "name": name, "arguments": arguments }),
    }
}

/// One card, in the frontend's vocabulary.
///
/// `answers` is what *this panel* sent for a question card, not the host's copy: it is
/// empty while the card is pending, and it is carried for `submitting` and `applied` so
/// the card can still show the user what they chose after the widgets are gone. The
/// verdict fields stay approval-only, exactly as they were.
fn interaction_view(interaction: &Interaction) -> Value {
    let (state, verdict, refusal, answers) = match interaction.state() {
        InteractionState::Pending => ("pending", None, None, &[][..]),
        InteractionState::Submitting { answer } => {
            let (verdict, answers) = sent_view(answer);
            ("submitting", verdict, None, answers)
        }
        InteractionState::Applied { answer } => {
            let (verdict, answers) = sent_view(answer);
            ("applied", verdict, None, answers)
        }
        InteractionState::Refused { reason } => ("refused", None, Some(reason.as_str()), &[][..]),
    };
    json!({
        "id": interaction.id(),
        "sessionId": interaction.session_id(),
        "kind": interaction.kind().as_str(),
        "state": state,
        "verdict": verdict,
        "refusalReason": refusal,
        "actionable": interaction.is_actionable(),
        "toolName": interaction.tool_name(),
        "detail": interaction.detail(DISPLAY_LOCALE),
        "questions": interaction.questions().iter().map(question_view).collect::<Vec<_>>(),
        "answers": answers.iter().map(answer_view).collect::<Vec<_>>(),
        "questionCount": interaction.question_count(),
    })
}

/// What a settled answer consisted of, split into the two fields the frontend reads.
///
/// An approval carries a verdict and no answers; a question carries answers and no
/// verdict. Both live in one match so a new kind of answer cannot be half-projected.
///
/// @param answer - what was sent.
/// @returns the `verdict` field and the answers to show, in that order.
fn sent_view(answer: &SentAnswer) -> (Option<&'static str>, &[QuestionAnswer]) {
    match answer {
        SentAnswer::Approval(verdict) => (Some(verdict.wire()), &[]),
        SentAnswer::Question(answers) => (None, answers.as_slice()),
    }
}

/// One question, in the frontend's vocabulary.
///
/// Every optional field is written as an explicit `null` rather than omitted, because
/// the frontend's type says `string | null` and a missing key would be a different type
/// to it than an absent heading.
fn question_view(question: &Question) -> Value {
    json!({
        "id": question.id,
        "header": question.header,
        "question": question.question,
        "detail": question.detail,
        "options": question
            .options
            .iter()
            .map(|option| json!({ "label": option.label, "description": option.description }))
            .collect::<Vec<_>>(),
        "multiSelect": question.multi_select,
    })
}

/// One answer this panel sent, in the frontend's vocabulary.
///
/// `custom` is `null` rather than a missing key, matching the `custom?: string` the
/// frontend sends and the nullability it reads back.
fn answer_view(answer: &QuestionAnswer) -> Value {
    json!({ "id": answer.id, "selected": answer.selected, "custom": answer.custom })
}

/// What became of an outbound request, in the frontend's vocabulary.
fn delivery_view(delivery: &Delivery) -> Value {
    match delivery {
        Delivery::Idle => json!({ "state": "idle", "status": null }),
        Delivery::Sending => json!({ "state": "sending", "status": "已提交，等待 Harness 确认…" }),
        Delivery::Accepted => json!({ "state": "accepted", "status": null }),
        Delivery::Failed { reason } => {
            json!({ "state": "failed", "status": format!("未发送：{reason}") })
        }
    }
}

/// The model and permission catalogs, in the frontend's vocabulary.
fn options_view(options: &SessionOptions) -> Value {
    json!({
        "models": options
            .models
            .iter()
            .map(|model| json!({
                "provider": model.provider,
                "providerName": model.provider_name,
                "id": model.id,
                "name": model.name,
                "description": model.description,
                "efforts": model
                    .efforts
                    .iter()
                    .map(|effort| json!({
                        "id": effort.id,
                        "name": effort.name,
                        "description": effort.description,
                    }))
                    .collect::<Vec<_>>(),
                "defaultEffort": model.default_effort,
            }))
            .collect::<Vec<_>>(),
        "current": options.current_model.as_ref().map(|(provider, model)| json!({
            "provider": provider,
            "model": model,
            "reasoningEffort": options.current_effort,
        })),
        "permissions": options
            .permissions
            .iter()
            .map(|permission| json!({
                "value": permission.value,
                "name": permission.name,
                "description": permission.description,
            }))
            .collect::<Vec<_>>(),
        "permission": options.current_permission,
    })
}

/// The conversation statistics, in the frontend's vocabulary.
fn stats_view(stats: &Stats) -> Value {
    json!({
        "turns": stats.turns,
        "steps": stats.steps,
        "tokensPerSecond": stats.tokens_per_second,
        "totalTokens": stats.total_tokens,
        "cacheHitPercent": stats.cache_hit_percent,
        "contextTokens": stats.context_tokens,
        "contextLimit": stats.context_limit,
    })
}

/// The wire name for one theme preference.
fn theme_name(theme: crate::app::theme::Preference) -> &'static str {
    match theme {
        crate::app::theme::Preference::System => "system",
        crate::app::theme::Preference::Light => "light",
        crate::app::theme::Preference::Dark => "dark",
    }
}

// ── commands ─────────────────────────────────────────────────────────────────

/// `submit` — send the user's text to the conversation this panel follows.
///
/// @param session - the session.
/// @param sink - where the request goes.
/// @param text - the user's text; blank input is refused by the session.
/// @param workspace_id - the workspace the panel chose for a new conversation, if any.
/// @param pinned_workspace - the workspace this window last chose: the ladder's P0 rung,
///   which the shell owns and the panel cannot see.
/// @returns the delivery, in the frontend's vocabulary.
#[must_use]
pub fn submit(
    session: &mut Session,
    sink: &mut dyn FrameSink,
    text: &str,
    workspace_id: Option<&str>,
    pinned_workspace: Option<&str>,
) -> Value {
    let workspace_id = create_workspace(session, workspace_id, pinned_workspace);
    let delivery = session.send_prompt(text, workspace_id.as_deref(), sink);
    json!({ "delivery": delivery_view(&delivery) })
}

/// Which workspace a new conversation is created in.
///
/// The panel's own choice wins while it still names one of the host's workspaces — the
/// picker only offers those, so a value the list no longer has is a stale snapshot rather
/// than an instruction. Everything else falls to the ladder in [`crate::app::workspace`],
/// which validates its own rungs: P0 is this window's own choice, P3 is the most recently
/// active workspace, and `None` is P4 — nothing to go on, so the panel has to ask the user.
///
/// The panel sends its choice when it has one and usually has none in the new-conversation
/// state, so without this the ladder is never consulted and a submit fails with "choose a
/// workspace" while the host is listing several.
///
/// @param session - the session, which holds the host's lists.
/// @param chosen - what the panel sent, if anything.
/// @param pinned - this window's P0 rung, if any.
/// @returns the workspace to ask for, or `None` when only the user can decide.
fn create_workspace(session: &Session, chosen: Option<&str>, pinned: Option<&str>) -> Option<String> {
    let follow = session.follow();
    let workspaces = follow.workspaces();
    chosen
        .filter(|id| !id.trim().is_empty())
        .filter(|id| workspaces.iter().any(|workspace| workspace.workspace_id == *id))
        .map(str::to_owned)
        // Nothing is on screen in the new-conversation state, so the ladder's P1 (the running
        // conversation's directory) cannot apply: this is P0, then P3, then the user.
        .or_else(|| workspace::to_create_in(pinned, None, false, workspaces, follow.sessions()))
}

/// `cancel` — stop the turn being generated.
///
/// @returns whether a cancel was sent, and its delivery state.
#[must_use]
pub fn cancel(session: &mut Session, sink: &mut dyn FrameSink) -> Value {
    let accepted = session.cancel_turn(sink);
    json!({ "accepted": accepted, "delivery": delivery_view(session.cancel_delivery()) })
}

/// `answer_approval` — settle one approval card.
///
/// @param interaction_id - which card.
/// @param allow - grant this one call, or refuse it.
/// @returns whether an answer was sent (the session refuses stale, resolved, or
///   question cards).
#[must_use]
pub fn answer_approval(
    session: &mut Session,
    sink: &mut dyn FrameSink,
    interaction_id: &str,
    allow: bool,
) -> Value {
    let verdict = if allow { ApprovalVerdict::AllowOnce } else { ApprovalVerdict::Reject };
    json!({ "sent": session.answer_interaction(interaction_id, verdict, sink) })
}

/// `answer_question` — settle one question card with what the user chose.
///
/// A thin call, like [`answer_approval`]: the caps, the validation and the frame all
/// belong to the session, so the bridge cannot grow a second opinion about what a
/// complete answer is.
///
/// @param interaction_id - which card.
/// @param answers - one answer per question, `selected` holding option **labels**.
/// @returns whether an answer was sent (the session refuses stale, resolved,
///   over-large, and incomplete answers, logging why).
#[must_use]
pub fn answer_question(
    session: &mut Session,
    sink: &mut dyn FrameSink,
    interaction_id: &str,
    answers: Vec<QuestionAnswer>,
) -> Value {
    json!({ "sent": session.answer_question(interaction_id, answers, sink) })
}

/// `select_session` — follow a different conversation.
///
/// @returns whether the attach request was written.
#[must_use]
pub fn select_session(session: &mut Session, sink: &mut dyn FrameSink, session_id: &str) -> Value {
    json!({ "accepted": session.choose_conversation(session_id, sink) })
}

/// `start_new` — go back to "new conversation" mode, dropping the followed one.
#[must_use]
pub fn start_new(session: &mut Session, sink: &mut dyn FrameSink) -> Value {
    session.start_new_conversation(sink);
    json!({})
}

/// `pin` — pin one conversation (or stop pinning), and remember it across runs.
///
/// The workspace field in the pin file is the session pin's **projection**: pinning a
/// conversation rewrites it to that conversation's own workspace, so the file can never
/// hold a pinned session and a different pinned workspace at once.
///
/// @param session_id - which one, or `None`.
/// @param pinned_path - where the pin is remembered; `None` is "nowhere to write".
/// @returns whether a request was written, and the resulting workspace projection.
#[must_use]
pub fn pin(
    session: &mut Session,
    sink: &mut dyn FrameSink,
    session_id: Option<&str>,
    pinned_path: Option<&std::path::Path>,
) -> Value {
    let accepted = session.pin_conversation(session_id.map(str::to_owned), sink);
    let mut workspace: Option<String> = None;
    if let Some(path) = pinned_path {
        let mut pinned = crate::app::pinned::Pinned::load(path);
        pinned.session = session.pinned_conversation().map(str::to_owned);
        pinned.workspace = pinned
            .session
            .as_deref()
            .and_then(|id| conversation_workspace(session, id));
        workspace = pinned.workspace.clone();
        pinned.save(path);
    }
    json!({ "accepted": accepted, "pinnedWorkspace": workspace })
}

/// `pin_workspace` — pin the default directory for a new conversation.
///
/// This is a *new-conversation-state* action by design: pinning a workspace
/// leaves the current conversation (with a `session/detach`) and clears the
/// session pin — the two fields can never disagree.
///
/// @param workspace_id - which workspace, or `None` to stop pinning one.
/// @param pinned_path - where the pin is remembered; `None` is "nowhere to write".
/// @returns the resulting workspace projection.
#[must_use]
pub fn pin_workspace(
    session: &mut Session,
    sink: &mut dyn FrameSink,
    workspace_id: Option<&str>,
    pinned_path: Option<&std::path::Path>,
) -> Value {
    // Leaving the current conversation is told to the host first: the detach
    // is what keeps the approval ownership honest.
    session.start_new_conversation(sink);
    let mut workspace: Option<String> = None;
    if let Some(path) = pinned_path {
        let mut pinned = crate::app::pinned::Pinned::load(path);
        pinned.session = None;
        pinned.workspace = workspace_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        workspace = pinned.workspace.clone();
        pinned.save(path);
    }
    json!({ "pinnedWorkspace": workspace })
}

/// `on_show` — what a summon means for the conversation: **every summon lands on a
/// fresh new-conversation state; the pin is the user's way of continuing a specific
/// conversation instead.**
///
/// - a pinned conversation that the panel is not currently following is
///   re-attached (the pin is the continuation escape hatch);
/// - no pin: `start_new_conversation` — the detach leaves the previous
///   conversation honestly, nothing is created until the user submits.
///
/// The dispatcher calls this on the *hotkey* show path only: a show the host
/// asks for (an approval, say) is the host's intent, not the user's summon.
///
/// @returns an empty result — the frame the session may have written is the
///   outcome, not this value.
#[must_use]
pub fn on_show(session: &mut Session, sink: &mut dyn FrameSink) -> Value {
    match session.pinned_conversation().map(str::to_owned) {
        Some(pinned) if session.follow().session_id() == Some(pinned.as_str()) => {}
        Some(pinned) => {
            session.pin_conversation(Some(pinned), sink);
        }
        None => {
            session.start_new_conversation(sink);
        }
    }
    json!({})
}

/// The workspace a conversation belongs to, when both lists say so.
///
/// @param session - the session, which holds both lists.
/// @param session_id - the conversation's id.
/// @returns its workspace id, or `None` when the cwd or the workspace is
///   unknown — an absent projection is absent, never guessed.
fn conversation_workspace(session: &Session, session_id: &str) -> Option<String> {
    let cwd = session
        .conversations()
        .iter()
        .find(|summary| summary.session_id == session_id)
        .and_then(|summary| summary.cwd.as_deref())?;
    session
        .workspaces()
        .iter()
        .find(|workspace| workspace.path == cwd)
        .map(|workspace| workspace.workspace_id.clone())
}

/// `request_workspaces` — ask the host for the workspace list the picker draws.
#[must_use]
pub fn request_workspaces(session: &mut Session, sink: &mut dyn FrameSink) -> Value {
    json!({ "accepted": session.request_workspaces(sink) })
}

/// `select_model` — apply one model (and reasoning effort) to the followed session.
#[must_use]
pub fn select_model(
    session: &mut Session,
    sink: &mut dyn FrameSink,
    provider: &str,
    model: &str,
    effort: Option<&str>,
) -> Value {
    session.select_model(provider, model, effort, sink);
    json!({})
}

/// `set_permission` — apply one permission preset to the followed session.
#[must_use]
pub fn set_permission(session: &mut Session, sink: &mut dyn FrameSink, value: &str) -> Value {
    session.set_permission(value, sink);
    json!({})
}

/// `dismiss_interaction` — drop one card locally, without answering it.
///
/// @returns whether a card was dropped.
#[must_use]
pub fn dismiss_interaction(session: &mut Session, interaction_id: &str) -> Value {
    json!({ "dropped": session.dismiss_interaction(interaction_id) })
}

/// `dismiss_handoff` — clear the "go to the Harness window" notice.
#[must_use]
pub fn dismiss_handoff(session: &mut Session) -> Value {
    session.dismiss_handoff();
    json!({})
}

/// `log` — put one frontend breadcrumb on the record.
///
/// Written to the **marker**, not stderr: the host swallows the sidecar's
/// stderr, so a line there would be exactly as invisible as the problem the
/// frontend is trying to report. The marker is the one surface a developer
/// (or a test) reads afterwards.
#[must_use]
pub fn log(sink: &mut dyn FrameSink, line: &str) -> Value {
    sink.mark(line);
    json!({})
}

/// `set_preferences` — apply what the settings page changed.
///
/// The validation rules are the shell's own, and they are strict on purpose: a
/// preference is remembered across runs, so a value this build cannot honour must
/// be refused here rather than quietly becoming something else on the next start.
///
/// @param preferences - what is already set; the incoming values sit on top.
/// @param incoming - `{theme?, keepOpen?, hotkey?, language?}`.
/// @returns the updated preferences, or an error naming the refused value.
pub fn apply_preferences(mut preferences: Preferences, incoming: &Value) -> Result<Preferences, String> {
    if let Some(theme) = incoming.get("theme").and_then(Value::as_str) {
        let theme = theme.trim();
        if ["system", "light", "dark"].iter().any(|known| known.eq_ignore_ascii_case(theme)) {
            preferences.theme = Some(crate::app::theme::Preference::from_name(Some(theme)));
        } else {
            return Err(format!("读不懂这个主题：{theme}"));
        }
    }
    if let Some(keep_open) = incoming.get("keepOpen").and_then(Value::as_bool) {
        preferences.keep_open = Some(keep_open);
    }
    if let Some(hotkey) = incoming.get("hotkey").and_then(Value::as_str) {
        let hotkey = hotkey.trim();
        validate_hotkey(hotkey)?;
        preferences.hotkey = Some(hotkey.to_owned());
    }
    if let Some(language) = incoming.get("language").and_then(Value::as_str) {
        // Refused rather than remapped, for the same reason an unknown theme is: a
        // language this build does not ship cannot be honoured, and quietly becoming
        // `zh` would make the settings page look like it worked.
        match crate::app::language::Language::known(Some(language)) {
            Some(language) => preferences.language = Some(language),
            None => return Err(format!("读不懂这个语言：{language}")),
        }
    }
    Ok(preferences)
}

/// Whether an accelerator the settings page recorded is one this process may hold.
///
/// Two rules, both load-bearing:
///
/// - the accelerator must be one this build can read back
///   ([`crate::runtime::hotkey::parse_accelerator`]);
/// - a **bare** letter, digit, or punctuation key is refused: a global hotkey is
///   grabbed from the whole desktop, so binding `M` would take the letter M away
///   from every application. Function keys are exempt — nobody "types" an F13 by
///   accident.
///
/// @param spec - the accelerator text, in the host's spelling.
/// @returns `Ok(())`, or a message the settings page can show.
pub fn validate_hotkey(spec: &str) -> Result<(), String> {
    let Some(parsed) = crate::runtime::hotkey::parse_accelerator(spec) else {
        return Err(format!("读不懂这个快捷键：{spec}"));
    };
    if parsed.mods.is_empty() && is_bare_typing_key(parsed.key) {
        return Err("裸按键会被整个桌面占用，请加上修饰键（Ctrl / Alt / Cmd / Shift）".to_owned());
    }
    Ok(())
}

/// The keys that would steal typing from every application if grabbed bare.
fn is_bare_typing_key(key: global_hotkey::hotkey::Code) -> bool {
    use global_hotkey::hotkey::Code as K;
    matches!(
        key,
        K::KeyA | K::KeyB | K::KeyC | K::KeyD | K::KeyE | K::KeyF | K::KeyG | K::KeyH | K::KeyI
            | K::KeyJ | K::KeyK | K::KeyL | K::KeyM | K::KeyN | K::KeyO | K::KeyP | K::KeyQ
            | K::KeyR | K::KeyS | K::KeyT | K::KeyU | K::KeyV | K::KeyW | K::KeyX | K::KeyY
            | K::KeyZ
            | K::Digit0 | K::Digit1 | K::Digit2 | K::Digit3 | K::Digit4 | K::Digit5 | K::Digit6
            | K::Digit7 | K::Digit8 | K::Digit9
            | K::Backquote | K::Minus | K::Equal | K::BracketLeft | K::BracketRight
            | K::Backslash | K::Semicolon | K::Quote | K::Comma | K::Period | K::Slash
    )
}
