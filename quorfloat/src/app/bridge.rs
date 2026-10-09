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
use crate::app::preferences::Preferences;
use crate::app::session::interaction::{ApprovalVerdict, Interaction, InteractionState};
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
fn interaction_view(interaction: &Interaction) -> Value {
    let (state, verdict, refusal) = match interaction.state() {
        InteractionState::Pending => ("pending", None, None),
        InteractionState::Submitting { verdict } => ("submitting", Some(verdict.wire()), None),
        InteractionState::Applied { verdict } => ("applied", Some(verdict.wire()), None),
        InteractionState::Refused { reason } => ("refused", None, Some(reason.as_str())),
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
        "questions": interaction.questions(),
        "questionCount": interaction.question_count(),
    })
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
/// @param workspace_id - the workspace to create in, when the panel is in
///   "new conversation" mode and the user has picked one.
/// @returns the delivery, in the frontend's vocabulary.
#[must_use]
pub fn submit(
    session: &mut Session,
    sink: &mut dyn FrameSink,
    text: &str,
    workspace_id: Option<&str>,
) -> Value {
    let delivery = session.send_prompt(text, workspace_id, sink);
    json!({ "delivery": delivery_view(&delivery) })
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
/// @param session_id - which one, or `None`.
/// @param pinned_path - where the pin is remembered; `None` is "nowhere to write".
/// @returns whether a request was written.
#[must_use]
pub fn pin(
    session: &mut Session,
    sink: &mut dyn FrameSink,
    session_id: Option<&str>,
    pinned_path: Option<&std::path::Path>,
) -> Value {
    let accepted = session.pin_conversation(session_id.map(str::to_owned), sink);
    if let Some(path) = pinned_path {
        // Loaded rather than rebuilt so a workspace pin written elsewhere survives
        // a session pin.
        let mut pinned = crate::app::pinned::Pinned::load(path);
        pinned.session = session.pinned_conversation().map(str::to_owned);
        pinned.save(path);
    }
    json!({ "accepted": accepted })
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
/// stderr, so a line there would be exactly as invisible as the bug the
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
/// @param incoming - `{theme?, keepOpen?, hotkey?}`.
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
    Ok(preferences)
}

/// Whether an accelerator the settings page recorded is one this process may hold.
///
/// Two rules, both from the egui settings page and both load-bearing:
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::session::test_support::{
        approval_open, follow_a_conversation, identity, RecordingSink,
    };
    use crate::ipc::rpc::Inbound;

    /// The shell view the snapshot tests read through.
    fn view() -> ShellView {
        ShellView {
            settings: WindowSettings::default(),
            preferences: Preferences::default(),
            hotkey_requested: Some("Alt+Space".to_owned()),
            hotkey_held: Some("Alt+Space".to_owned()),
            hotkey_registered: true,
            hotkey_reason: None,
            visible: false,
            height: Height::new(),
        }
    }

    /// A session that has handshaken and attached to `session-1`, with its sink.
    fn followed() -> (Session, RecordingSink) {
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        follow_a_conversation(&mut session, &mut sink);
        (session, sink)
    }

    #[test]
    fn the_snapshot_carries_the_shells_own_facts() {
        let session = Session::new(identity());
        let value = snapshot(&session, &view());
        assert_eq!(value["settings"]["width"], 640.0);
        assert_eq!(value["settings"]["theme"], "system");
        assert_eq!(value["hotkey"]["requested"], "Alt+Space");
        assert_eq!(value["hotkey"]["registered"], true);
        assert_eq!(value["height"]["target"], 0.0);
        assert_eq!(value["height"]["capped"], false);
        assert_eq!(value["window"]["visible"], false, "the shell reports the dispatcher's truth");
        assert_eq!(value["session"]["ready"], false, "no handshake yet");
    }

    #[test]
    fn the_snapshot_projects_transcript_entries_and_the_live_stream() {
        let (mut session, mut sink) = followed();
        session.on_frame(
            Inbound::Notification {
                method: "session/snapshot".to_owned(),
                params: Some(serde_json::json!({
                    "sessionId": "session-1", "generation": 3, "cursor": 2, "hasMore": false,
                    "records": [
                        {"type": "event", "event": {"type": "user/message", "seq": 0, "time": 1,
                         "data": {"role": "user", "content": [{"type": "text", "text": "帮我看看"}]}}},
                        {"type": "event", "event": {"type": "assistant/message", "seq": 1, "time": 2,
                         "data": {"message": {"role": "assistant",
                            "content": [{"type": "text", "text": "看到了"}]}}}},
                        {"type": "event", "event": {"type": "tool/result", "seq": 2, "time": 3,
                         "data": {"message": {"role": "tool", "toolCallId": "call-1", "isError": false,
                            "content": [{"type": "text", "text": "exit=0"}]}}}},
                    ],
                })),
            },
            &mut sink,
        );
        session.on_frame(
            Inbound::Notification {
                method: "session/stream".to_owned(),
                params: Some(serde_json::json!({
                    "sessionId": "session-1", "generation": 3,
                    "frame": {"type": "chunk", "revision": 5, "index": 1,
                              "chunk": {"type": "text-delta", "index": 0, "text": "正在写"}},
                })),
            },
            &mut sink,
        );

        let value = snapshot(&session, &view());
        let entries = value["transcript"]["entries"].as_array().expect("entries");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0]["kind"], "user");
        assert_eq!(entries[0]["text"], "帮我看看");
        assert_eq!(entries[1]["kind"], "assistant");
        assert_eq!(entries[1]["blocks"][0]["kind"], "text");
        assert_eq!(entries[2]["kind"], "tool");
        assert_eq!(entries[2]["name"], Value::Null, "no call id was remembered for this result");
        assert_eq!(entries[2]["isError"], false);
        assert_eq!(value["transcript"]["streaming"], true);
        assert_eq!(value["transcript"]["live"]["kind"], "assistant");
        assert_eq!(value["transcript"]["live"]["streaming"], true);
        assert_eq!(value["transcript"]["live"]["blocks"][0]["text"], "正在写");
        assert_eq!(value["session"]["following"]["sessionId"], "session-1");
    }

    #[test]
    fn submit_sends_the_prompt_and_reports_its_delivery() {
        let (mut session, mut sink) = followed();
        let result = submit(&mut session, &mut sink, "第一句", None);
        assert_eq!(result["delivery"]["state"], "sending");
        let frame = sink.frames.last().expect("a prompt request");
        assert_eq!(frame["method"], "session/prompt");
        assert_eq!(frame["params"]["sessionId"], "session-1");
        assert_eq!(frame["params"]["text"], "第一句");
    }

    #[test]
    fn answering_an_approval_sends_the_wire_verdict() {
        let (mut session, mut sink) = followed();
        session.on_frame(approval_open("a-1"), &mut sink);
        let value = snapshot(&session, &view());
        assert_eq!(value["interactions"][0]["id"], "a-1");
        assert_eq!(value["interactions"][0]["kind"], "approval");
        assert_eq!(value["interactions"][0]["state"], "pending");
        assert_eq!(value["interactions"][0]["actionable"], true);
        assert_eq!(value["interactions"][0]["toolName"], "bash");
        assert_eq!(value["interactions"][0]["detail"], "escalate sandbox to danger-full-access: write the file");

        let result = answer_approval(&mut session, &mut sink, "a-1", false);
        assert_eq!(result["sent"], true);
        let frame = sink.frames.last().expect("an answer");
        assert_eq!(frame["method"], "interaction/answer");
        assert_eq!(frame["params"]["interactionId"], "a-1");
        assert_eq!(frame["params"]["answer"]["outcome"], "rejected");
        assert_eq!(snapshot(&session, &view())["interactions"][0]["state"], "submitting");
    }

    #[test]
    fn the_settings_page_cannot_set_a_bare_typing_key() {
        let error = validate_hotkey("M").expect_err("a bare letter is refused");
        assert!(error.contains("裸按键"), "{error}");
        assert!(validate_hotkey("F13").is_ok(), "a function key is exempt");
        assert!(validate_hotkey("Cmd+Shift+K").is_ok());
        assert!(validate_hotkey("Alt+Banana").is_err(), "an unreadable accelerator is refused");
    }

    #[test]
    fn the_settings_page_cannot_set_a_theme_this_build_does_not_know() {
        let error = apply_preferences(
            Preferences::default(),
            &serde_json::json!({ "theme": "purple" }),
        )
        .expect_err("an unknown theme is refused, not guessed");
        assert!(error.contains("purple"), "{error}");
    }

    #[test]
    fn known_preferences_are_applied_on_top_of_what_was_set() {
        let mut preferences = Preferences { keep_open: Some(false), ..Preferences::default() };
        preferences = apply_preferences(
            preferences,
            &serde_json::json!({ "theme": "dark", "keepOpen": true, "hotkey": "Cmd+Shift+K" }),
        )
        .expect("all three are known values");
        assert_eq!(preferences.theme, Some(crate::app::theme::Preference::Dark));
        assert_eq!(preferences.keep_open, Some(true));
        assert_eq!(preferences.hotkey.as_deref(), Some("Cmd+Shift+K"));
    }

    #[test]
    fn pinning_writes_the_pin_file_and_keeps_a_workspace_pin() {
        let path = std::env::temp_dir().join(format!("quorfloat-bridge-pin-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let (mut session, mut sink) = followed();
        {
            let mut pinned = crate::app::pinned::Pinned::default();
            pinned.workspace = Some("workspace-9".to_owned());
            pinned.save(&path);
        }
        let result = pin(&mut session, &mut sink, Some("session-1"), Some(&path));
        assert_eq!(
            result["accepted"], false,
            "pinning the already-attached conversation needs no request",
        );
        assert_eq!(
            session.pinned_conversation(),
            Some("session-1"),
            "the pin itself is recorded regardless",
        );
        let saved = crate::app::pinned::Pinned::load(&path);
        assert_eq!(saved.session.as_deref(), Some("session-1"), "the session pin is remembered");
        assert_eq!(saved.workspace.as_deref(), Some("workspace-9"), "the workspace pin survives");
        let _ = std::fs::remove_file(&path);
    }
}
