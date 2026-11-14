//! Sample data for gallery scenes: a believable WackCode session, so a scene shows real lengths
//! and real rhythm rather than lorem ipsum.

use std::{collections::HashSet, path::Path};

use serde_json::json;
use wack_core::{Event, Outcome, Usage};

use crate::{
    composer::{ComposerData, Input, Meter, Waiting},
    conversation::Conversation,
    shell::ProjectInfo,
    sidebar::{ChatRow, ChatState, ProjectRow, SidebarData},
    transcript::Item,
    work::{self, ReplyCache},
};

/// Which chat the sidebar shows as selected.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Selected {
    None,
    Finished,
    Working,
}

pub fn sidebar(selected: Selected) -> SidebarData {
    let chat = |title: &'static str, age: &'static str, state, this: Selected| ChatRow {
        id: title.into(),
        title: title.into(),
        age: age.into(),
        state,
        selected: selected == this && this != Selected::None,
    };
    let closed = |name: &'static str, chats: usize, pinned: bool| ProjectRow {
        id: name.into(),
        name: name.into(),
        chats: (0..chats)
            .map(|_| chat("", "", ChatState::Idle, Selected::None))
            .collect(),
        open: false,
        pinned,
    };
    SidebarData {
        pinned: vec![
            ProjectRow {
                id: "wackcode".into(),
                name: "wackcode".into(),
                open: true,
                pinned: true,
                chats: vec![
                    chat("Fix the SSE decoder tail", "now", ChatState::Idle, Selected::Finished),
                    chat("Honour Retry-After", "8s", ChatState::Working, Selected::Working),
                    chat("Port the theme derivation", "2h", ChatState::Idle, Selected::None),
                    chat("Tool search for other hosts", "1d", ChatState::Unread, Selected::None),
                    chat("Kill the whole process group", "3d", ChatState::Idle, Selected::None),
                ],
            },
            closed("wackchatter", 3, true),
            closed("TokenTrail", 2, true),
        ],
        projects: vec![
            closed("Media-Logger", 4, false),
            closed("Godot-Bench", 1, false),
            closed("godot-terraria", 6, false),
            closed("cargo-cleaner", 0, false),
            closed("WackWebsite", 2, false),
        ],
    }
}

pub fn project() -> ProjectInfo {
    ProjectInfo {
        name: "wackcode".into(),
        path: "~/Documents/GitHub/wackcode".into(),
        branch: Some("3.0-Remake".into()),
    }
}

fn drawn(text: Option<&'static str>, placeholder: &'static str) -> Input {
    Input::Drawn { text: text.map(Into::into), placeholder: placeholder.into() }
}

fn meter(label: &'static str) -> Option<Meter> {
    // Compact is offered from a quarter full (shell::meter), so not at these levels.
    Some(Meter { label: label.into(), high: false, can_compact: false })
}

pub fn composer_empty(roomy: bool) -> ComposerData {
    ComposerData {
        input: drawn(None, crate::app::NEW_CHAT_PLACEHOLDER),
        model: "GLM-5.3-Flash".into(),
        effort: Some("Medium".into()),
        meter: None,
        working: false,
        waiting: Vec::new(),
        roomy,
        menu: None,
    }
}

/// After a turn, with `input` holding the next message.
pub fn composer_after_turn(input: Input) -> ComposerData {
    ComposerData { input, meter: meter("14% context"), ..composer_empty(false) }
}

pub fn composer_working() -> ComposerData {
    ComposerData {
        input: drawn(None, crate::chat::FOLLOW_UP_PLACEHOLDER),
        meter: meter("21% context"),
        working: true,
        waiting: vec![
            Waiting { text: "Check the date form against RFC 9110 too.".into(), steering: true },
            Waiting { text: "Also cap the wait at 60 seconds, whatever the server asks for.".into(), steering: false },
        ],
        ..composer_empty(false)
    }
}

/// Two connections, as Settings › Providers lists them.
pub fn connections() -> Vec<wack_core::Connection> {
    use wack_core::{Connection, Model, WireFormat};
    let model = |id: &str, window: Option<u64>| Model { id: id.into(), context_window: window };
    vec![
        Connection {
            id: "c1".into(),
            name: "Work".into(),
            format: WireFormat::AnthropicMessages,
            base_url: "https://api.anthropic.com".into(),
            models: vec![model("claude-opus-5-5", Some(1_000_000)), model("claude-sonnet-5-5", Some(1_000_000))],
        },
        Connection {
            id: "c2".into(),
            name: "Local".into(),
            format: WireFormat::OpenaiChat,
            base_url: "http://localhost:1234/v1".into(),
            models: vec![model("qwen/qwen3-coder-30b", Some(128_000))],
        },
    ]
}

/// The model menu open over the composer, from [`connections`].
pub fn model_menu() -> crate::composer::MenuData {
    use crate::composer::{Choice, MenuData, MenuGroup, MenuItem};
    let groups = connections()
        .into_iter()
        .map(|connection| MenuGroup {
            title: Some(connection.name.clone().into()),
            items: connection
                .models
                .iter()
                .map(|model| MenuItem {
                    label: crate::shell::model_name(&model.id).into(),
                    detail: model.context_window.map(|window| work::count(window).into()),
                    selected: model.id == "claude-opus-5-5",
                    choice: Choice::Model { connection: connection.id.clone(), model: model.id.clone() },
                })
                .collect(),
        })
        .collect();
    MenuData { kind: crate::app::Menu::Model, groups }
}

pub const NEXT_MESSAGE: &str = "Now make retries honour Retry-After";

pub const FINISHED_TITLE: &str = "Fix the SSE decoder tail";
pub const WORKING_TITLE: &str = "Honour Retry-After";

const CWD: &str = "/Users/you/Documents/GitHub/wackcode";
const START: u64 = 1_800_000_000_000;

fn call(conversation: &mut Conversation, id: &str, name: &str, input: serde_json::Value, result: Option<&str>, at: u64) {
    conversation.apply(&Event::ToolCall { id: id.into(), name: name.into(), input }, at);
    if let Some(content) = result {
        conversation.apply(&Event::ToolResult { id: id.into(), name: name.into(), content: content.into(), is_error: false }, at);
    }
}

const REPLY: &str = "Fixed. **The decoder now flushes what it has buffered when the stream ends**, so a final event without its blank line is delivered instead of dropped.

- `Decoder::finish` returns the pending event, and `HttpProvider::stream` calls it once the body is exhausted.
- A stream that stops halfway through a field still yields nothing: half an event is never guessed at.

```rust
#[test]
fn flushes_an_unterminated_final_event() {
    let mut decoder = Decoder::default();
    decoder.push(b\"data: {\\\"done\\\":true}\");
    // No blank line: the stream just ends.
    assert_eq!(decoder.finish().unwrap().data, \"{\\\"done\\\":true}\");
    assert_eq!(decoder.finish(), None);
}
```

All six SSE tests pass, including the `0`-byte body case.";

/// The first turn of the sample chat, run through the same code as a real one.
fn finished(conversation: &mut Conversation) {
    conversation.send("The SSE decoder drops the last event when a stream ends without a blank line. Fix it and add a test.", START);
    conversation.apply(
        &Event::Thinking("The decoder only emits on a blank line, so a final event with no terminator never leaves the buffer.".into()),
        START,
    );
    let file = "x\n".repeat(214);
    call(conversation, "read-1", "read", json!({ "path": format!("{CWD}/crates/wack-llm/src/sse.rs") }), Some(&file), START);
    call(conversation, "grep-1", "grep", json!({ "pattern": "finish", "path": "crates/wack-llm" }), Some("a\nb\nc"), START);
    call(
        conversation,
        "edit-1",
        "edit",
        json!({ "path": "crates/wack-llm/src/sse.rs", "edits": [{
            "old_text": "    /// Called once the body is exhausted.\n    pub fn finish(&mut self) {}",
            "new_text": "    /// Called once the body is exhausted.\n    pub fn finish(&mut self) -> Option<Event> {\n        if self.data.is_empty() {\n            return None;\n        }\n        Some(self.take_event())\n    }"
        }] }),
        Some("Edited crates/wack-llm/src/sse.rs (1 change)."),
        START,
    );
    call(conversation, "bash-1", "bash", json!({ "command": "cargo test -p wack-llm sse" }), Some("test result: ok. 6 passed; 0 failed"), START);
    conversation.apply(&Event::Text(REPLY.into()), START);
    let usage = Usage { input: 1_400, output: 2_800, cache_read: 11_000, cache_write: 0 };
    conversation.apply(&Event::Reply { usage: Some(usage) }, START);
    conversation.finish(&Ok(Outcome::Done), START + 19_000);
}

fn items(conversation: &Conversation, open: &[&str], now: u64) -> Vec<Item> {
    let open: HashSet<String> = open.iter().map(|key| key.to_string()).collect();
    work::items(conversation, Path::new(CWD), &open, now, &mut ReplyCache::default())
}

/// A finished turn with its work log open on the edit's diff.
pub fn finished_turn() -> Vec<Item> {
    let mut conversation = Conversation::default();
    finished(&mut conversation);
    items(&conversation, &["turn-1", "edit-1"], START + 30_000)
}

/// The same chat with a second turn under way.
pub fn working_turn() -> Vec<Item> {
    let mut conversation = Conversation::default();
    finished(&mut conversation);
    let now = START + 60_000;
    conversation.send("Make retries honour Retry-After when the server sends it, in seconds or as a date.", now - 8_000);
    conversation.apply(
        &Event::Thinking("Retry-After can be a number of seconds or an HTTP date; both have to cap at the backoff ceiling.".into()),
        now,
    );
    let file = "x\n".repeat(142);
    call(&mut conversation, "read-2", "read", json!({ "path": "crates/wack-llm/src/retry.rs" }), Some(&file), now);
    call(&mut conversation, "read-3", "read", json!({ "path": "crates/wack-llm/src/error.rs" }), Some(&"x\n".repeat(96)), now);
    call(
        &mut conversation,
        "edit-2",
        "edit",
        json!({ "path": "crates/wack-llm/src/retry.rs", "edits": [{ "old_text": "a\nb\nc", "new_text": "a\nB\nC\nD\nc" }] }),
        Some("Edited."),
        now,
    );
    call(&mut conversation, "bash-2", "bash", json!({ "command": "cargo test -p wack-llm retry" }), None, now);
    conversation.apply(&Event::Text("Retry-After is now read from both 429 and 503 responses. Running the retry tests to check the date form".into()), now);
    items(&conversation, &[], now)
}
