//! Helpers the session tests share.
//!
//! They used to be copied into each test module; the approval tests and the handshake
//! tests want the same scripted host, the same recording sink, and the same idea of what
//! a signed-in session looks like. One copy, in one place, so a change to the fake host
//! cannot leave one suite testing against a shape the other no longer uses.

use std::collections::VecDeque;

use serde_json::{Value, json};

use crate::app::session::{FrameSink, FrameSource, HotkeyReport, Identity, Session};
use crate::ipc::rpc::Inbound;

/// A sink that keeps frames in memory instead of writing them.
#[derive(Default)]
pub(super) struct RecordingSink {
    pub(super) frames: Vec<Value>,
    pub(super) logs: Vec<String>,
    pub(super) marks: Vec<String>,
}

impl FrameSink for RecordingSink {
    fn send(&mut self, frame: &Value) -> std::io::Result<()> {
        self.frames.push(frame.clone());
        Ok(())
    }

    fn log(&mut self, line: &str) {
        self.logs.push(line.to_owned());
    }

    fn mark(&mut self, line: &str) {
        self.marks.push(line.to_owned());
    }
}

/// A source that hands out prepared frames and then reports end of stream.
pub(super) struct ScriptedSource {
    frames: VecDeque<Inbound>,
}

impl ScriptedSource {
    pub(super) fn new(frames: Vec<Inbound>) -> Self {
        Self { frames: frames.into() }
    }
}

impl FrameSource for ScriptedSource {
    fn next_frame(&mut self) -> Option<Inbound> {
        self.frames.pop_front()
    }
}

pub(super) fn identity() -> Identity {
    Identity {
        quorfloat_version: "0.0.1".to_owned(),
        platform: "darwin".to_owned(),
        arch: "arm64".to_owned(),
        hotkey: HotkeyReport { requested: "Alt+Space".to_owned(), registered: true },
    }
}

pub(super) fn request(id: i64, method: &str) -> Inbound {
    Inbound::Request { id: json!(id), method: method.to_owned(), params: None }
}

pub(super) fn notification(method: &str, params: Value) -> Inbound {
    Inbound::Notification { method: method.to_owned(), params: Some(params) }
}

/// The host's answer to a good `hello`, trimmed to the fields this side reads.
pub(super) fn hello_ok() -> Inbound {
    Inbound::Response {
        id: json!(1),
        outcome: Ok(json!({"protocol": "quorfloat/1", "hostVersion": "0.2.0-rc.2",
                           "sessionId": "quorfloat-1-abc"})),
    }
}

pub(super) fn conversation_snapshot() -> serde_json::Value {
    json!({
        "sessionId": "session-1", "generation": 1, "cursor": 1, "hasMore": false,
        "records": [
            // The two wrappers are the real asymmetry: `user/message` carries its
            // content directly, the assistant's is inside `message`.
            {"type": "event", "event": {"type": "user/message", "seq": 0, "time": 1,
             "data": {"role": "user", "content": [{"type": "text", "text": "帮我看看"}]}}},
            {"type": "event", "event": {"type": "assistant/message", "seq": 1, "time": 2,
             "data": {"message": {"role": "assistant",
                "content": [{"type": "text", "text": "看到了"}]}}}},
        ],
    })
}

pub(super) fn approval_open(id: &str) -> Inbound {
    notification(
        "interaction/open",
        json!({
            "interactionId": id,
            "sessionId": "session-1",
            "kind": "approval",
            "payload": {
                "toolName": "bash",
                "callId": "call-1",
                "reason": "escalate sandbox to danger-full-access: write the file",
            },
        }),
    )
}

/// Deliver frames without the handshake, which these tests do not exercise.
pub(super) fn deliver(session: &mut Session, frames: Vec<Inbound>, sink: &mut RecordingSink) {
    for frame in frames {
        session.on_frame(frame, sink);
    }
}

pub(super) fn follow_a_conversation(session: &mut Session, sink: &mut RecordingSink) {
    session.on_frame(hello_ok(), sink);
    session.pump_follow_at(0, sink);
    let list = sink.frames.last().expect("a list request")["id"].clone();
    session.on_frame(
        Inbound::Response {
            id: list,
            outcome: Ok(json!({"items": [
                {"sessionId": "older", "updatedAt": 1, "cwd": "/work/project"},
                {"sessionId": "session-1", "updatedAt": 9, "cwd": "/work/project"},
            ]})),
        },
        sink,
    );
    let attach = sink.frames.last().expect("an attach request")["id"].clone();
    session.on_frame(
        Inbound::Response { id: attach, outcome: Ok(json!({"sessionId": "session-1", "generation": 3})) },
        sink,
    );
}
