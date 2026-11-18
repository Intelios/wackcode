//! Sample data for gallery scenes: a believable WackCode session, so a scene shows real lengths
//! and real rhythm rather than lorem ipsum.

use std::{collections::HashSet, path::{Path, PathBuf}};

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
        look: Default::default(),
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
        ultra_effort: false,
        mode: wack_core::AgentMode::Build,
        key: "scene".into(),
        meter: None,
        working: false,
        waiting: Vec::new(),
        roomy,
        menu: None,
        slash: None,
        hint: None,
        question: None,
        todos: None,
        activity: None,
        todos_open: false,
        waiting_open: false,
        tasks: Vec::new(),
        tasks_open: false,
        foreground: false,
        agents: 0,
    }
}

/// After a turn, with `input` holding the next message.
pub fn composer_after_turn(input: Input) -> ComposerData {
    ComposerData { input, meter: meter("14%"), ..composer_empty(false) }
}

pub fn composer_working() -> ComposerData {
    ComposerData {
        input: drawn(None, crate::chat::FOLLOW_UP_PLACEHOLDER),
        meter: meter("21%"),
        working: true,
        activity: Some(("Working  ·  8s".into(), Some("Running cargo test -p wack-llm retry".into()))),
        waiting: vec![
            Waiting { text: "Check the date form against RFC 9110 too.".into(), steering: true },
            Waiting { text: "Also cap the wait at 60 seconds, whatever the server asks for.".into(), steering: false },
        ],
        ..composer_empty(false)
    }
}

/// A model that reasons, listing what current Anthropic models accept.
pub fn reasoning_model() -> wack_core::Model {
    use wack_core::Reasoning::*;
    wack_core::Model { efforts: vec![Off, Low, Medium, High, Xhigh, Max], ..wack_core::Model::new("claude-opus-5-5", Some(1_000_000)) }
}

/// Two connections, as Settings › Providers lists them.
pub fn connections() -> Vec<wack_core::Connection> {
    use wack_core::{Connection, Model, WireFormat};
    let model = |id: &str, window: Option<u64>| Model { efforts: reasoning_model().efforts, ..Model::new(id, window) };
    vec![
        Connection {
            id: "c1".into(),
            name: "Work".into(),
            format: WireFormat::AnthropicMessages,
            base_url: "https://api.anthropic.com".into(),
            models: vec![model("claude-opus-5-5", Some(1_000_000)), model("claude-sonnet-5-5", Some(1_000_000))],
            enabled: true,
        },
        Connection {
            id: "c2".into(),
            name: "Local".into(),
            format: WireFormat::OpenaiChat,
            base_url: "http://localhost:1234/v1".into(),
            models: vec![model("qwen/qwen3-coder-30b", Some(128_000))],
            enabled: true,
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

/// A chat's background commands: the dev server and a watch running, the test suite failed.
pub fn tasks() -> wack_core::tasks::Tasks {
    use wack_core::tasks::{Kind, Status, TaskInfo, Tasks, Why, now_millis};
    let tasks = Tasks::scratch();
    let now = now_millis();
    let info = |id: &str, command: &str, description: &str, kind, started: u64, ended: Option<u64>, status| TaskInfo {
        id: id.into(),
        command: command.into(),
        description: description.into(),
        kind,
        started_at: now - started,
        ended_at: ended.map(|ended| now - ended),
        status,
        background: Some(Why::Asked),
        log: format!("/tmp/{id}.log").into(),
        bytes: 0,
        last_line: String::new(),
    };
    tasks.insert_fixture(
        info("p8q2w7z0", "npm test", "Run the test suite", Kind::Shell, 260_000, Some(168_000), Status::Failed { code: Some(1), signal: None }),
        TEST_OUTPUT,
    );
    tasks.insert_fixture(
        info("k3v9x2m1", "npm run dev", "Start the dev server", Kind::Shell, 262_000, None, Status::Running),
        DEV_OUTPUT,
    );
    tasks.insert_fixture(
        info("w5n1c8d4", "tail -f dev.log | grep --line-buffered -i error", "Dev server errors", Kind::Watch, 90_000, None, Status::Running),
        "",
    );
    tasks
}

const DEV_OUTPUT: &str = "\u{1b}[32m  VITE v6.2.0\u{1b}[0m  ready in 412 ms\n\n  ➜  Local:   http://localhost:5173/\n  ➜  Network: use --host to expose\n  ➜  press h + enter to show help\n12:04:31 [vite] page reload src/retry.ts\n12:05:02 [vite] hmr update /src/App.tsx\n";
const TEST_OUTPUT: &str = " ✓ src/sse.test.ts (14 tests) 38ms\n ✓ src/decoder.test.ts (31 tests) 52ms\n ✗ src/retry.test.ts (9 tests | 2 failed) 61ms\n   × honours Retry-After as an HTTP date\n   × caps the wait at the backoff ceiling\n\n Test Files  1 failed | 23 passed (24)\n      Tests  2 failed | 212 passed (214)\n";

/// A chat that started two commands in the background, then woke when the tests failed.
pub fn task_turns() -> Vec<Item> {
    let mut conversation = Conversation::default();
    conversation.send("Start the dev server, and run the whole test suite while I look at the page.", START);
    call(
        &mut conversation,
        "bash-a",
        "bash",
        json!({ "command": "npm run dev", "description": "Start the dev server", "run_in_background": true }),
        Some("Started in the background (ID: k3v9x2m1)."),
        START + 2_000,
    );
    call(
        &mut conversation,
        "bash-b",
        "bash",
        json!({ "command": "npm test", "description": "Run the test suite", "run_in_background": true }),
        Some("Started in the background (ID: p8q2w7z0)."),
        START + 3_000,
    );
    conversation.apply(
        &Event::Text("Both are running: the dev server is at http://localhost:5173, and the tests are going in the background. I'll tell you how they did when they finish.".into()),
        START + 5_000,
    );
    conversation.finish(&Ok(Outcome::Done), START + 6_000);
    conversation.wake(START + 98_000);
    conversation.apply(
        &Event::Noted {
            tag: "task:p8q2w7z0:end".into(),
            text: format!(
                "Background task p8q2w7z0 (`npm test`) failed: exit code 1 after 1m32s.\nLast lines:\n```\n{}```\nFull output: /tmp/p8q2w7z0.log",
                TEST_OUTPUT
            ),
        },
        START + 98_000,
    );
    conversation.apply(
        &Event::Text("The suite finished with **2 failures, both in `retry.test.ts`**: an HTTP-date `Retry-After` isn't parsed, and the wait isn't capped at the backoff ceiling. The other 212 pass. Want me to fix the retry parser?".into()),
        START + 104_000,
    );
    conversation.finish(&Ok(Outcome::Done), START + 105_000);
    items(&conversation, &[], START + 110_000)
}

/// A settled turn that filed every kind of suggestion, the next message having started one of
/// them: the tray shows every state a card can be in.
pub fn suggestions_turn() -> Vec<Item> {
    use crate::suggestions::{Card, Kind};
    let mut conversation = Conversation::default();
    conversation.send("Fix the SSE decoder tail, and flag anything else worth a look while you're in there.", START);
    let file = "x\n".repeat(60);
    call(
        &mut conversation,
        "read-s",
        "read",
        json!({ "path": format!("{CWD}/crates/wack-llm/src/sse.rs") }),
        Some(&file),
        START,
    );
    let suggest = |conversation: &mut Conversation, id: &str, kind: &str, title: &str, brief: &str| {
        call(
            conversation,
            id,
            "suggest_task",
            json!({ "kind": kind, "title": title, "description": brief }),
            Some("Saved."),
            START,
        );
    };
    suggest(
        &mut conversation,
        "sug-1",
        "bug",
        "Empty queue row flashes on send",
        "`submit` clears the composer before the queued row mounts, so the text blinks out and back. Keep it until the row draws.",
    );
    suggest(
        &mut conversation,
        "sug-2",
        "polish",
        "Diff headers could carry the file icon",
        "A small affordance that would help scan a long set of edits.",
    );
    suggest(
        &mut conversation,
        "sug-3",
        "improvement",
        "A keybinding for the tools pane",
        "⌘⌥T feels like the free slot; check the keymap before offering it.",
    );
    suggest(
        &mut conversation,
        "sug-4",
        "polish",
        "Soften the backdrop under the tray",
        "The backdrop image competes with the cards on this screen. A stronger wash under the \
         tray, like the composer's, would settle it. This brief is long on purpose: dismissed \
         cards fold away behind a title and a Restore so they stay findable without cluttering \
         the tray, and a bug keeps the danger tone whatever the theme.",
    );
    suggest(
        &mut conversation,
        "sug-5",
        "improvement",
        "Remember the last pane layout per chat",
        "The layout lives in the session already; it just isn't persisted.",
    );
    conversation.apply(&Event::Text(REPLY.into()), START);
    conversation.finish(&Ok(Outcome::Done), START + 30_000);
    // The next message started the third suggestion: it carries the generated tail, so the card
    // reads started here and the bubble shows the suggestion, never the tail.
    let started = Card {
        id: "sug-3".into(),
        kind: Kind::Improvement,
        title: "A keybinding for the tools pane".into(),
        description: "⌘⌥T feels like the free slot; check the keymap before offering it.".into(),
    };
    conversation.send(&crate::suggestions::compose(&started, None), START + 60_000);
    conversation.finish(&Ok(Outcome::Done), START + 61_000);
    // Dismissals, the opened chat and the queue arrive from the chat's saved metadata.
    conversation.suggestion_view.dismissed.insert("sug-5".into());
    conversation.suggestion_view.queued.insert("sug-2".into());
    conversation.suggestion_view.links.insert("sug-4".into(), ("chat-9".into(), "Soften the backdrop".into()));
    items(&conversation, &[], START + 70_000)
}

/// The composer under the tasks chat, with the background commands listed (`open`).
pub fn composer_tasks(open: bool) -> ComposerData {
    ComposerData {
        tasks: crate::shell::task_lines(&tasks()),
        tasks_open: open,
        ..composer_after_turn(drawn(None, crate::chat::FOLLOW_UP_PLACEHOLDER))
    }
}

/// The agent waiting on a slow build, which ⌘B would move to the background.
pub fn composer_foreground() -> ComposerData {
    ComposerData {
        working: true,
        foreground: true,
        agents: 0,
        activity: Some(("Working  ·  2m 04s".into(), Some("Running cargo build --release".into()))),
        tasks: crate::shell::task_lines(&tasks()),
        ..composer_after_turn(drawn(None, crate::chat::FOLLOW_UP_PLACEHOLDER))
    }
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

/// The agent checking a form in the browser: each step in words, the last one still running.
pub fn browser_turn() -> Vec<Item> {
    let mut conversation = Conversation::default();
    let now = START + 60_000;
    conversation.send("Open the sign-up page on the dev server, fill it in with test details and send it. Tell me what the console says.", now - 20_000);
    let outline = "document \"Sign up\"\n  textbox \"Email\" [ref_3]\n  textbox \"Password\" [ref_4] value=[redacted]\n  button \"Create account\" [ref_7]";
    call(&mut conversation, "b-1", "tool_search", json!({ "query": "browser" }), Some("Loaded 15 tools."), now);
    call(&mut conversation, "b-2", "browser_navigate", json!({ "url": "http://localhost:5173/signup?invite=4f9c2a7e1b3d8c6f0a5e9d2b7c4f1a8e3d6b9c2f" }), Some("Tab 1 (yours): \"Sign up\" localhost:5173/signup"), now);
    call(&mut conversation, "b-3", "browser_snapshot", json!({}), Some(outline), now);
    call(&mut conversation, "b-4", "browser_fill", json!({ "fields": [{ "ref": "ref_3", "value": "test@example.com" }, { "ref": "ref_4", "value": "x" }] }), Some("Filled 2 fields."), now);
    call(&mut conversation, "b-5", "browser_act", json!({ "action": "click", "ref": "ref_7" }), Some("Clicked ref_7. A new page loaded: refs from earlier snapshots are gone."), now);
    call(&mut conversation, "b-6", "browser_wait", json!({ "text": "Welcome" }), Some("“Welcome” is on the page."), now);
    call(&mut conversation, "b-7", "browser_screenshot", json!({}), Some("A 1280×800 screenshot."), now);
    call(&mut conversation, "b-8", "browser_console", json!({ "errors_only": true }), None, now);
    items(&conversation, &[], now)
}

/// A chat opened with a skill: the `/pdf` bubble, the skill loaded, the reply. `open` shows the
/// prompt the agent got beneath the bubble.
pub fn skill_turn(open: bool) -> Vec<Item> {
    use wack_core::skills::{CommandUse, Kind};
    let mut conversation = Conversation::default();
    let skill = "/Users/me/.agents/skills/pdf/SKILL.md";
    let expanded = format!(
        "<skill name=\"pdf\" location=\"{skill}\">\nReferences are relative to /Users/me/.agents/skills/pdf.\n\n# Filling PDF forms\n\nList the form's fields with scripts/fields.py first, then fill them from the data the user gives. Never flatten a form unless asked.\n</skill>\n\nfill in the W-9 in forms/ from my details in notes.md"
    );
    conversation.send_command(&expanded, CommandUse { name: "pdf".into(), args: "fill in the W-9 in forms/ from my details in notes.md".into(), kind: Kind::Skill }, START);
    call(&mut conversation, "read-s", "read", json!({ "path": skill }), Some("# Filling PDF forms"), START + 2_000);
    call(&mut conversation, "read-n", "read", json!({ "path": "notes.md" }), Some(&"x\n".repeat(24)), START + 4_000);
    call(&mut conversation, "bash-f", "bash", json!({ "command": "python3 ~/.agents/skills/pdf/scripts/fill.py forms/w9.pdf" }), Some("Filled 11 fields."), START + 9_000);
    conversation.apply(&Event::Text("Filled the W-9's eleven fields from notes.md and saved it as `forms/w9-filled.pdf`. The signature line is left for you.".into()), START + 12_000);
    conversation.finish(&Ok(Outcome::Done), START + 14_000);
    items(&conversation, if open { &["prompt-0", "turn-1"] } else { &["turn-1"] }, START + 20_000)
}

/// The `/` menu's rows for `/re`, as a chat would offer them.
pub fn slash_rows() -> Vec<crate::slash_menu::SlashRow> {
    use crate::slash_menu::SlashRow;
    use wack_core::skills::Kind;
    let row = |name: &str, hint: Option<&str>, description: &str, source: &str, kind| SlashRow {
        name: name.to_string().into(),
        hint: hint.map(|hint| hint.to_string().into()),
        description: description.to_string().into(),
        source: source.to_string().into(),
        kind,
    };
    vec![
        row("/review", Some("<file>"), "Review a file the way the team does, before changing anything", "Your commands", Kind::Template),
        row("/release-notes", Some("[since]"), "Draft release notes from the commits since a tag", "Your skills", Kind::Skill),
        row("/reverse-engineer-anything", None, "Reverse engineer native, managed, Electron and packaged targets", "Your skills", Kind::Skill),
        row("/compact", Some("[focus]"), "Summarise older conversation context", "WackCode", Kind::Builtin),
    ]
}

/// A planning turn that asked a question and is waiting on the next.
pub fn asking_turn() -> Vec<Item> {
    let mut conversation = Conversation::default();
    let now = START + 60_000;
    conversation.send("Plan how drafts should survive a crash.", now - 20_000);
    call(&mut conversation, "read-q", "read", json!({ "path": "crates/wack-ui/src/chat.rs" }), Some(&"x\n".repeat(390)), now - 15_000);
    call(
        &mut conversation,
        "ask-1",
        "ask_user_question",
        json!({ "questions": [{ "id": "scope", "header": "Scope", "question": "Should drafts of new chats survive too?",
            "options": [{ "label": "Every draft", "description": "New chats and replies alike." },
                        { "label": "Replies only", "description": "A new chat's draft is short-lived." }] }] }),
        Some("The user answered your questions:\n\n**Scope** — Should drafts of new chats survive too?\n- Every draft"),
        now - 9_000,
    );
    call(&mut conversation, "ask-2", "ask_user_question", json!({ "questions": questions_json() }), None, now);
    items(&conversation, &["ask-1"], now)
}

fn questions_json() -> serde_json::Value {
    json!([
        { "id": "store", "header": "Storage", "question": "Where should a draft be kept while you type?",
          "options": [
            { "label": "Beside the chat (Recommended)", "description": "One small file per chat in the session folder, written as you pause." },
            { "label": "In state.json", "description": "With the other chat settings; simplest, but rewrites the whole file." },
            { "label": "Memory only", "description": "Nothing on disk: a crash loses the draft." }
          ] },
        { "id": "when", "header": "Timing", "question": "When should it be written?",
          "options": [
            { "label": "After a pause", "description": "Half a second after the last key." },
            { "label": "On every key", "description": "Never loses a letter; more writes." }
          ], "multiSelect": false }
    ])
}

/// The questions the card is asking in the scenes.
pub fn questions() -> Vec<wack_core::ask::Question> {
    wack_core::ask::normalize(&json!({ "questions": questions_json() })).unwrap_or_default()
}

/// The question card, first question answered with a preset, as Plan or Ultra Plan shows it.
pub fn question(ultra: bool) -> crate::question::QuestionData {
    let questions = questions();
    let drafts = vec![
        crate::question::Draft { selected: vec!["Beside the chat (Recommended)".into()], custom: String::new() },
        crate::question::Draft::default(),
    ];
    crate::question::QuestionData {
        questions,
        tab: 0,
        drafts,
        wrap_up: ultra,
        other: drawn(None, "Other — type a custom answer"),
        focus: None,
        nods: 0,
    }
}

const PLAN: &str = "## Keep drafts across a crash

1. **Save as you pause.** `ChatSession` writes the composer's text to `drafts/<chat>.txt` half a second after the last key.
2. **Restore on open.** `ChatSession::load` reads the file back into the composer, then removes it.
3. **Clear on send.** Sending, or emptying the field, deletes the file.
4. **Test it.** A `tempfile` test for each step, and a gallery scene with a restored draft.

Assumptions: new chats keep their draft too; nothing is written for an empty field.";

/// A planning turn that ended with a proposal awaiting review.
pub fn plan_turn(ultra: bool, confirming_discard: bool) -> Vec<Item> {
    let mut conversation = Conversation::default();
    let now = START + 60_000;
    let mode = if ultra { "ultraplan" } else { "plan" };
    conversation.send("Plan how drafts should survive a crash.", now - 40_000);
    call(&mut conversation, "read-p", "read", json!({ "path": "crates/wack-ui/src/chat.rs" }), Some(&"x\n".repeat(390)), now - 35_000);
    call(&mut conversation, "grep-p", "grep", json!({ "pattern": "set_text" }), Some(&"x\n".repeat(12)), now - 30_000);
    call(&mut conversation, "plan-p", "plan_mode_complete", json!({ "plan": PLAN }), Some("Plan submitted for review."), now - 2_000);
    conversation.apply(&Event::State { key: "plan".into(), value: json!({ "mode": mode, "plan": PLAN }) }, now - 1_000);
    conversation.finish(&Ok(Outcome::Done), now);
    conversation.plan_view.confirming_discard = confirming_discard;
    items(&conversation, &[], now)
}

/// A todo list part-way through a run: one task done before it, one during it, one under way.
pub fn todos() -> crate::todo_panel::TodoData {
    use wack_core::todo::{Status, Task, TodoList};
    let task = |id, subject: &str, status, active: Option<&str>, blocked_by: Vec<u64>| Task {
        id,
        subject: subject.into(),
        description: None,
        active_form: active.map(Into::into),
        status,
        blocked_by,
    };
    let list = TodoList {
        tasks: vec![
            task(1, "Read how the composer saves its text", Status::Completed, None, vec![]),
            task(2, "Write the draft beside the chat", Status::Completed, None, vec![]),
            task(3, "Restore the draft when the chat opens", Status::InProgress, Some("restoring drafts"), vec![2]),
            task(4, "Delete the draft once it is sent", Status::Pending, None, vec![2]),
            task(5, "Add the gallery scene", Status::Pending, None, vec![]),
        ],
        next_id: 6,
    };
    let faded = [1].into_iter().collect();
    crate::todo_panel::TodoData { view: crate::todo_panel::view(&list, &faded, false), busy: true }
}

/// The title strip's tabs, with `active` in front (or the new-chat tab when `None`).
pub fn strip(active: Option<&'static str>) -> crate::shell::StripData {
    let tab = |title: &'static str, state| crate::shell::TabData {
        id: title.into(),
        title: title.into(),
        look: Default::default(),
        active: active == Some(title),
        state,
    };
    crate::shell::StripData {
        tabs: vec![
            tab(FINISHED_TITLE, ChatState::Idle),
            tab(WORKING_TITLE, ChatState::Working),
            tab("Tool search for other hosts", ChatState::Unread),
        ],
        new_chat: active.is_none(),
        commands: Some([
            crate::run_button::CommandButtonData::idle(wack_core::CommandKind::Build, None),
            crate::run_button::CommandButtonData::idle(wack_core::CommandKind::Run, None),
        ]),
        command_editor: None,
    }
}

pub const BUILD_SESSION: &str = "\x1b[1;32m    Checking\x1b[0m wack-core v3.0.0-dev.0 (/Users/you/Code/wackcode/crates/wack-core)\r\n\
\x1b[1;32m    Checking\x1b[0m wack-ui v3.0.0-dev.0 (/Users/you/Code/wackcode/crates/wack-ui)\r\n\
\x1b[1;31merror[E0308]\x1b[0m\x1b[1m: mismatched types\x1b[0m\r\n\
  \x1b[1;34m-->\x1b[0m crates/wack-ui/src/app.rs:142:9\r\n\
   \x1b[1;34m|\x1b[0m\r\n\
\x1b[1;34m142|\x1b[0m         self.store.set_project_command(id, kind, &raw)\r\n\
   \x1b[1;34m|\x1b[0m                                                  \x1b[1;31m^^^^ expected `Option<&str>`, found `&String`\x1b[0m\r\n\
\x1b[1;31merror\x1b[0m: could not compile `wack-ui` (lib) due to 1 previous error\r\n";

pub const RUN_SESSION: &str = "\x1b[1;32m    Finished\x1b[0m `dev` profile [unoptimized + debuginfo] target(s) in 1.18s\r\n\
\x1b[1;32m     Running\x1b[0m `target/debug/dev-server --port 4190`\r\n\
\x1b[36m[server]\x1b[0m listening on \x1b[1mhttp://127.0.0.1:4190\x1b[0m\r\n\
\x1b[36m[watch]\x1b[0m  watching crates/ for changes...\r\n\
\x1b[32m  ready\x1b[0m  200 OK GET /api/health (1.2ms)\r\n";

/// What the Changes pane lists after the SSE fix.
pub fn changed_files() -> Vec<wack_core::git::ChangedFile> {
    use wack_core::git::{ChangedFile, FileStatus};
    let file = |path: &str, status, lines| ChangedFile { path: path.into(), old_path: None, status, lines };
    vec![
        file("README.md", FileStatus::Modified, Some((2, 2))),
        file("crates/wack-llm/src/sse.rs", FileStatus::Modified, Some((14, 3))),
        file("crates/wack-llm/src/stream.rs", FileStatus::Modified, Some((3, 1))),
        file("crates/wack-llm/tests/tail.rs", FileStatus::Untracked, Some((41, 0))),
        file("notes/decoder.txt", FileStatus::Deleted, Some((0, 12))),
    ]
}

pub const CHANGES_SELECTED: &str = "crates/wack-llm/src/sse.rs";

pub const CHANGES_DIFF: &str = "diff --git a/crates/wack-llm/src/sse.rs b/crates/wack-llm/src/sse.rs
--- a/crates/wack-llm/src/sse.rs
+++ b/crates/wack-llm/src/sse.rs
@@ -38,9 +38,12 @@ impl Decoder {
     /// Feeds a chunk of the body. Returns the events it completed.
     pub fn feed(&mut self, chunk: &[u8]) -> Vec<Event> {
         self.buffer.extend_from_slice(chunk);
-        self.drain()
+        let mut events = self.drain();
+        events.retain(|event| !event.data.is_empty());
+        events
     }
 
@@ -71,8 +74,19 @@ impl Decoder {
     /// Called once the body is exhausted.
-    pub fn finish(&mut self) {}
+    pub fn finish(&mut self) -> Option<Event> {
+        // A server may close the stream without the blank line that ends the last event:
+        // whatever is buffered is still an event.
+        if !self.buffer.is_empty() {
+            self.line(&std::mem::take(&mut self.buffer));
+        }
+        if self.data.is_empty() {
+            return None;
+        }
+        Some(self.take_event())
+    }
 
     fn take_event(&mut self) -> Event {
         let data = std::mem::take(&mut self.data);
";

/// A terminal that ran the tests: a prompt, coloured output, and the prompt again.
pub const TERMINAL_SESSION: &str = "\x1b[1;32m➜\x1b[0m  \x1b[1;36mwackcode\x1b[0m \x1b[1;34mgit:(\x1b[31m3.0-Remake\x1b[34m)\x1b[0m cargo test -p wack-llm sse\r\n\
\x1b[1;32m   Compiling\x1b[0m wack-llm v3.0.0-dev.0 (/Users/you/Documents/GitHub/wackcode/crates/wack-llm)\r\n\
\x1b[1;32m    Finished\x1b[0m `test` profile [unoptimized + debuginfo] target(s) in 2.41s\r\n\
\x1b[1;32m     Running\x1b[0m unittests src/lib.rs (target/debug/deps/wack_llm-5c1e0d)\r\n\
\r\n\
running 4 tests\r\n\
test sse::tests::splits_events_across_chunks ... \x1b[32mok\x1b[0m\r\n\
test sse::tests::ignores_comments ... \x1b[32mok\x1b[0m\r\n\
test sse::tests::keeps_a_tail_without_a_blank_line ... \x1b[32mok\x1b[0m\r\n\
test sse::tests::drops_empty_data ... \x1b[32mok\x1b[0m\r\n\
\r\n\
test result: \x1b[32mok\x1b[0m. 4 passed; 0 failed; 0 ignored; 0 measured; 61 filtered out; finished in 0.01s\r\n\
\r\n\
\x1b[1;32m➜\x1b[0m  \x1b[1;36mwackcode\x1b[0m \x1b[1;34mgit:(\x1b[31m3.0-Remake\x1b[34m) \x1b[33m✗\x1b[0m git status --short\r\n\
 \x1b[31mM\x1b[0m README.md\r\n\
 \x1b[31mM\x1b[0m crates/wack-llm/src/sse.rs\r\n\
 \x1b[31mM\x1b[0m crates/wack-llm/src/stream.rs\r\n\
 \x1b[31mD\x1b[0m notes/decoder.txt\r\n\
\x1b[31m??\x1b[0m crates/wack-llm/tests/tail.rs\r\n\
\x1b[1;32m➜\x1b[0m  \x1b[1;36mwackcode\x1b[0m \x1b[1;34mgit:(\x1b[31m3.0-Remake\x1b[34m) \x1b[33m✗\x1b[0m ";

/// The gallery's backdrop, imported once into `target/gallery/backdrop/` through the app's own
/// import (so it has a softened copy too). A real photo saved as
/// `target/gallery/fixture-photo.jpg` is used when there is one; otherwise a dusk landscape is
/// drawn, so backdrop scenes render on any machine.
pub fn backdrop(texture: wack_core::Texture) -> crate::backdrop::Picture {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).unwrap_or(Path::new("."));
    let gallery = workspace.join("target/gallery");
    let folder = gallery.join("backdrop");
    let record = folder.join("backdrop.json");
    let photo = gallery.join("fixture-photo.jpg");
    let source = if photo.exists() { photo } else { gallery.join("fixture-dusk.png") };
    let imported = std::fs::read_to_string(&record)
        .ok()
        .and_then(|text| serde_json::from_str::<(String, wack_core::Backdrop)>(&text).ok())
        .filter(|(from, backdrop)| {
            *from == source.to_string_lossy() && folder.join(&backdrop.file).exists() && backdrop.shade.is_some()
        });
    let backdrop = match imported {
        Some((_, backdrop)) => backdrop,
        None => {
            let _ = std::fs::remove_dir_all(&folder);
            if !source.exists() {
                let (width, height) = (2400, 1500);
                let _ = std::fs::create_dir_all(&gallery);
                let picture = image::RgbImage::from_fn(width, height, |x, y| dusk(x as f32 / width as f32, y as f32 / height as f32));
                let _ = picture.save(&source);
            }
            let backdrop = wack_core::appearance::import_backdrop(&source, &folder).expect("the gallery's backdrop imports");
            let _ = std::fs::write(&record, serde_json::to_string(&(source.to_string_lossy(), &backdrop)).unwrap_or_default());
            backdrop
        }
    };
    let path = |file: String| folder.join(file).to_string_lossy().to_string().into();
    crate::backdrop::Picture {
        path: path(backdrop.file.clone()),
        soft_path: path(wack_core::appearance::soft_name(&backdrop.file)),
        width: backdrop.width,
        height: backdrop.height,
        focus: (0.5, 0.5),
        dim: wack_core::appearance::DEFAULT_DIM as f32 / 100.,
        blur: wack_core::appearance::DEFAULT_BLUR as f32,
        texture,
        shade: backdrop.shade.unwrap_or([0, 255]),
    }
}

/// One pixel of the dusk picture, at `u` across and `v` down.
fn dusk(u: f32, v: f32) -> image::Rgb<u8> {
    let mix = |a: [f32; 3], b: [f32; 3], t: f32| {
        let t = t.clamp(0., 1.);
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
    };
    // Night blue at the top, through violet, to a warm glow at the horizon.
    let (night, violet, glow) = ([0.09, 0.10, 0.24], [0.42, 0.27, 0.52], [0.98, 0.58, 0.34]);
    let mut colour = if v < 0.4 { mix(night, violet, v / 0.4) } else { mix(violet, glow, (v - 0.4) / 0.24) };
    // A low sun with a soft halo.
    let distance = (((u - 0.70) * 1.6).powi(2) + (v - 0.60).powi(2)).sqrt();
    colour = mix(colour, [1.0, 0.86, 0.62], (1. - distance / 0.25).max(0.).powi(2) * 0.8);
    if distance < 0.045 {
        colour = [1.0, 0.93, 0.78];
    }
    // Three ridges, each nearer one darker.
    let ridges = [(0.60, 0.035, [0.47, 0.31, 0.50], 3.1), (0.70, 0.05, [0.27, 0.18, 0.37], 1.7), (0.82, 0.06, [0.11, 0.09, 0.20], 1.1)];
    for (index, (base, height, ink, frequency)) in ridges.into_iter().enumerate() {
        let phase = index as f32 * 1.9;
        let wave = |scale: f32| (u * frequency * scale * std::f32::consts::TAU + phase * scale).sin();
        let line = base - height * (wave(1.) * 0.6 + wave(2.7) * 0.3 + wave(9.) * 0.1);
        if v > line {
            colour = mix(ink, [0.05, 0.04, 0.10], (v - line) / 0.6);
        }
    }
    // A fine grain, like film.
    let grain = ((u * 9137.1 + v * 3271.7).sin() * 43758.5).fract() * 0.02 - 0.01;
    image::Rgb(colour.map(|channel| ((channel + grain).clamp(0., 1.) * 255.).round() as u8))
}

/// Settings › Appearance in the theme the scene is drawn with. With `backdrop`, the library holds
/// it, shown, and the same picture again kept in view at its corner and remembering Solar.
pub fn appearance(backdrop: Option<crate::backdrop::Picture>, cx: &mut gpui::App) -> crate::appearance_page::AppearanceData {
    let theme = crate::theme::Theme::get(cx).input.theme;
    let pictures = backdrop.iter().flat_map(|picture| {
        [
            crate::appearance_page::Thumbnail { file: "shown.jpg".into(), thumb: picture.path.clone(), picture: picture.clone(), shown: true, theme: Some(theme) },
            crate::appearance_page::Thumbnail {
                file: "corner.jpg".into(),
                thumb: picture.path.clone(),
                picture: crate::backdrop::Picture { focus: (0., 0.), ..picture.clone() },
                shown: false,
                theme: Some(wack_core::ThemeName::Solar),
            },
        ]
    });
    crate::appearance_page::AppearanceData {
        mode: wack_core::Mode::System,
        theme,
        pictures: pictures.collect(),
        backdrop,
        importing: false,
        notice: None,
    }
}

// -------------------------------------------------------------------------------------------------
// Sub-agents
// -------------------------------------------------------------------------------------------------

const MAP_REPORT: &str = "Retry-After is read in one place, `crates/wack-llm/src/retry.rs:41` (`parse_retry_after`), and only for 429s. Three callers back off through it:\n\n- `HttpProvider::stream` (`crates/wack-llm/src/http.rs:212`) on a failed connect\n- `Compactor::summarise` (`crates/wack-agent/src/compact.rs:88`), which retries once\n- the title generator (`crates/wack-core/src/title.rs:57`), which gives up after one wait\n\nNone of them cap the wait: `Backoff::next` (`retry.rs:19`) is the only ceiling, and a server's Retry-After skips it.\n\n[agent a1 · explore · 14 tool calls · 38k tokens · 1m 12s]";

/// One agent as the registry has it, `started` and `ended` milliseconds before `now`.
#[allow(clippy::too_many_arguments)]
fn agent_info(id: &str, role: &str, description: &str, call: &str, status: wack_core::agents::Status, background: bool, now: u64, started: u64, ended: Option<u64>, tools: u32, tokens: u64, activity: Option<(&str, &str)>, session: std::path::PathBuf) -> wack_core::agents::AgentInfo {
    wack_core::agents::AgentInfo {
        id: id.into(),
        role: role.into(),
        description: description.into(),
        call_id: call.into(),
        round: 1,
        round_call_id: String::new(),
        session,
        model: if role == "general" { "claude-opus-5-5".into() } else { "claude-haiku-5-5".into() },
        edits: role == "general",
        started_at: now - started,
        ended_at: ended.map(|ended| now - ended),
        status,
        background,
        tool_calls: tools,
        context_tokens: tokens,
        activity: activity.map(|(tool, target)| wack_core::agents::Activity { tool: tool.into(), target: target.into() }),
    }
}

/// The agents a chat sent out to look before changing the retry logic: one has reported, two
/// are working (one in the background) and one waits for a place. `session` is where the second
/// one's work is saved.
pub fn agents(now: u64, session: std::path::PathBuf) -> Vec<wack_core::agents::AgentInfo> {
    use wack_core::agents::Status;
    let elsewhere = |id: &str| std::path::PathBuf::from(format!("/nowhere/{id}.jsonl"));
    vec![
        agent_info("a1", "explore", "Map the retry callers", "agent-1", Status::Done, false, now, 84_000, Some(12_000), 14, 38_400, None, elsewhere("a1")),
        agent_info("a2", "explore", "Find the backoff ceilings", "agent-2", Status::Running, false, now, 84_000, None, 9, 21_700, Some(("grep", "Retry-After")), session),
        agent_info("a3", "review", "Check the retry tests", "agent-3", Status::Running, true, now, 84_000, None, 6, 17_200, Some(("bash", "cargo test -p wack-llm retry")), elsewhere("a3")),
        agent_info("a4", "general", "Draft the date parser", "agent-4", Status::Queued, false, now, 84_000, None, 0, 0, None, elsewhere("a4")),
    ]
}

/// A turn that sent four agents out together, with the first one back.
pub fn agent_turn() -> Vec<Item> {
    let mut conversation = Conversation::default();
    let now = START + 90_000;
    conversation.send("Before I change the retry logic: find everything that depends on how we back off, and check the tests cover the date form.", START);
    conversation.apply(&Event::Thinking("Three separate questions; they can run side by side while the parser gets drafted.".into()), START + 4_000);
    let agent = |agent: &str, description: &str| json!({ "agent": agent, "description": description, "prompt": "…" });
    call(&mut conversation, "agent-1", "agent", agent("explore", "Map the retry callers"), Some(MAP_REPORT), START + 6_000);
    call(&mut conversation, "agent-2", "agent", agent("explore", "Find the backoff ceilings"), None, START + 6_000);
    call(
        &mut conversation,
        "agent-3",
        "agent",
        json!({ "agent": "review", "description": "Check the retry tests", "prompt": "…", "background": true }),
        Some("Agent a3 (review) is working in the background. Its report will arrive as a message when it finishes."),
        START + 6_000,
    );
    call(&mut conversation, "agent-4", "agent", agent("general", "Draft the date parser"), None, START + 6_000);
    conversation.agents = agents(now, "/nowhere/a2.jsonl".into()).into_iter().map(|info| (info.call_id.clone(), info)).collect();
    items(&conversation, &[], now)
}

/// Agents that ended badly, and a report from the background waking the chat.
pub fn agent_ends() -> Vec<Item> {
    use wack_core::agents::{Status, StoppedBy};
    let mut conversation = Conversation::default();
    let nowhere = |id: &str| std::path::PathBuf::from(format!("/nowhere/{id}.jsonl"));
    conversation.send("Review the retry change in the background, and port the date parser from the old client meanwhile.", START);
    call(
        &mut conversation,
        "agent-1",
        "agent",
        json!({ "agent": "review", "description": "Review the retry change", "prompt": "…", "background": true }),
        Some("Agent a1 (review) is working in the background. Its report will arrive as a message when it finishes."),
        START + 3_000,
    );
    call(&mut conversation, "agent-2", "agent", json!({ "agent": "general", "description": "Port the date parser", "prompt": "…" }), Some("The connection dropped twice and the agent gave up before it changed anything."), START + 4_000);
    conversation.apply(&Event::Text("The port didn't get anywhere: its agent lost the connection before changing a file. The review is still going; I'll pass on what it finds.".into()), START + 52_000);
    conversation.finish(&Ok(Outcome::Done), START + 53_000);
    conversation.wake(START + 140_000);
    conversation.apply(
        &Event::Noted {
            tag: "agent:a1".into(),
            text: "Agent a1 (review · Review the retry change) finished in the background. Its report:\n\n**Critical:** `retry.rs:44` trusts Retry-After without a ceiling, so a server asking for an hour stalls the chat for an hour.\n\n**Warning:** `retry.rs:52` parses HTTP dates in local time.\n\n[agent a1 · review · 11 tool calls · 26k tokens · 2m 16s]".into(),
        },
        START + 140_000,
    );
    conversation.apply(&Event::Text("The review found **one critical issue**: a server's Retry-After is trusted without a ceiling. Want me to cap it at the backoff limit?".into()), START + 146_000);
    conversation.finish(&Ok(Outcome::Done), START + 147_000);
    conversation.send("Yes, and look for anywhere else that reads it.", START + 200_000);
    call(&mut conversation, "agent-3", "agent", json!({ "agent": "explore", "description": "Find other Retry-After reads", "prompt": "…" }), None, START + 204_000);
    conversation.finish(&Ok(Outcome::Stopped), START + 231_000);
    conversation.send("Carry on.", START + 900_000);
    call(&mut conversation, "agent-4", "agent", json!({ "agent": "explore", "description": "Find other Retry-After reads", "prompt": "…" }), None, START + 902_000);
    conversation.finish(&Ok(Outcome::Done), START + 930_000);
    let now = START + 990_000;
    let mut list = vec![
        agent_info("a1", "review", "Review the retry change", "agent-1", Status::Done, true, now, 987_000, Some(850_000), 11, 26_100, None, nowhere("a1")),
        agent_info("a2", "general", "Port the date parser", "agent-2", Status::Failed { message: "The connection dropped twice.".into() }, false, now, 986_000, Some(940_000), 3, 8_800, None, nowhere("a2")),
        agent_info("a3", "explore", "Find other Retry-After reads", "agent-3", Status::Stopped { by: StoppedBy::User }, false, now, 786_000, Some(759_000), 5, 12_300, None, nowhere("a3")),
        agent_info("a4", "explore", "Find other Retry-After reads", "agent-4", Status::Interrupted, false, now, 88_000, Some(60_000), 7, 15_900, None, nowhere("a4")),
    ];
    conversation.agents = list.drain(..).map(|info| (info.call_id.clone(), info)).collect();
    let mut items = items(&conversation, &[], now);
    for item in &mut items {
        if let Item::Work(log) = item {
            log.open = true;
        }
    }
    items
}

/// The composer while the chat waits on its agents.
pub fn composer_agents() -> ComposerData {
    ComposerData {
        working: true,
        foreground: true,
        agents: 3,
        activity: Some(("Working  ·  1m 30s".into(), Some("Waiting on Explore · Find the backoff ceilings".into()))),
        ..composer_after_turn(drawn(None, crate::chat::FOLLOW_UP_PLACEHOLDER))
    }
}

/// The agents registry for the Agents pane, with the second agent's work saved under `folder`.
pub fn agents_registry(folder: &Path) -> wack_core::agents::Agents {
    use wack_core::llm::Block;
    use wack_core::{Message, Role};
    let now = wack_core::tasks::now_millis();
    let path = folder.join("a2.jsonl");
    let mut lines = vec![json!({ "type": "meta", "version": 1, "cwd": CWD, "created": now - 84_000 })];
    let mut push = |message: Message, ago: u64| lines.push(json!({ "type": "message", "message": message, "at": now - ago }));
    push(Message::user("Find every ceiling on how long a retry may wait: constants, config, and anything that clamps a Retry-After. Report each with file:line and whether a server's value can skip it."), 84_000);
    let assistant = |blocks: Vec<Block>| Message { role: Role::Assistant, content: blocks };
    let result = |id: &str, content: &str| Message { role: Role::User, content: vec![Block::ToolResult { tool_use_id: id.into(), content: content.into(), is_error: false, images: Vec::new() }] };
    let tool = |id: &str, name: &str, input: serde_json::Value| Block::ToolUse { id: id.into(), name: name.into(), input };
    push(assistant(vec![Block::Text { text: "Starting from the retry module, then everything that calls it.".into() }, tool("g1", "grep", json!({ "pattern": "Backoff|MAX_WAIT|ceiling", "path": "crates" }))]), 80_000);
    push(result("g1", "crates/wack-llm/src/retry.rs:12:const MAX_WAIT: Duration = Duration::from_secs(60);\ncrates/wack-llm/src/retry.rs:19:    pub fn next(&mut self) -> Duration {\ncrates/wack-core/src/title.rs:57:    let wait = Backoff::once();"), 79_000);
    push(assistant(vec![tool("r1", "read", json!({ "path": format!("{CWD}/crates/wack-llm/src/retry.rs") })), tool("r2", "read", json!({ "path": format!("{CWD}/crates/wack-core/src/title.rs"), "offset": 40, "limit": 40 }))]), 70_000);
    push(result("r1", &"x\n".repeat(118)), 69_000);
    push(result("r2", &"x\n".repeat(40)), 69_000);
    push(assistant(vec![Block::Text { text: "`MAX_WAIT` caps `Backoff::next` at 60 seconds, but `parse_retry_after` returns the server's value straight to the caller. Checking whether any caller clamps it.".into() }, tool("g2", "grep", json!({ "pattern": "Retry-After", "path": "crates" }))]), 41_000);
    let body: String = lines.iter().map(|line| format!("{line}\n")).collect();
    let _ = std::fs::create_dir_all(folder);
    let _ = std::fs::write(&path, body);
    let registry = wack_core::agents::Agents::new();
    for info in agents(now, path) {
        registry.insert_fixture(info);
    }
    registry
}

/// A still of a web page for the Browser pane's scenes, which have no engine: a site bar, a
/// heading, text and a sign-up form, drawn once into `target/gallery`.
pub fn page_picture(width: u32, height: u32) -> PathBuf {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).unwrap_or(Path::new("."));
    let gallery = workspace.join("target/gallery");
    let path = gallery.join(format!("fixture-page-{width}x{height}.png"));
    if path.exists() {
        return path;
    }
    let _ = std::fs::create_dir_all(&gallery);
    let unit = width as f32 / 100.;
    let inside = |x: u32, y: u32, (left, top, w, h): (f32, f32, f32, f32)| {
        let (x, y) = (x as f32 / unit, y as f32 / unit);
        x >= left && x < left + w && y >= top && y < top + h
    };
    let bar = (0., 0., 100., 9.);
    let logo = (6., 2.8, 14., 3.4);
    let links = [(62., 3.5, 8., 2.), (73., 3.5, 8., 2.), (84., 3.5, 9., 2.)];
    let heading = [(6., 16., 70., 6.), (6., 24., 48., 6.)];
    let lines = [(6., 35., 84., 1.8), (6., 39., 80., 1.8), (6., 43., 62., 1.8)];
    let fields = [(6., 52., 60., 7.), (6., 62., 60., 7.)];
    let button = (6., 73., 26., 7.);
    let picture = image::RgbImage::from_fn(width, height, |x, y| {
        let at = |area| inside(x, y, area);
        let edge = |(left, top, w, h): (f32, f32, f32, f32)| {
            at((left, top, w, h)) && !at((left + 0.3, top + 0.3, w - 0.6, h - 0.6))
        };
        let colour: [u8; 3] = if at(logo) {
            [32, 36, 52]
        } else if links.iter().any(|link| at(*link)) {
            [120, 126, 140]
        } else if at(bar) {
            [244, 245, 248]
        } else if heading.iter().any(|block| at(*block)) {
            [36, 40, 56]
        } else if lines.iter().any(|line| at(*line)) {
            [196, 200, 210]
        } else if fields.iter().any(|field| edge(*field)) {
            [176, 182, 196]
        } else if at(button) {
            [52, 104, 230]
        } else {
            [255, 255, 255]
        };
        image::Rgb(colour)
    });
    let _ = picture.save(&path);
    path
}
