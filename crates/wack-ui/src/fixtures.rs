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
        ultra_effort: false,
        mode: wack_core::AgentMode::Build,
        meter: None,
        working: false,
        waiting: Vec::new(),
        roomy,
        menu: None,
        question: None,
        todos: None,
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
    crate::todo_panel::TodoData { view: crate::todo_panel::view(&list, &faded, false), busy: true, folded: false }
}

/// The title strip's tabs, with `active` in front (or the new-chat tab when `None`).
pub fn strip(active: Option<&'static str>) -> crate::shell::StripData {
    let tab = |title: &'static str, state| crate::shell::TabData {
        id: title.into(),
        title: title.into(),
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
    }
}

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

/// Settings › Appearance as first opened: the default colours, and `backdrop` if given.
pub fn appearance(backdrop: Option<crate::backdrop::Picture>, cx: &mut gpui::App) -> crate::appearance_page::AppearanceData {
    use gpui::AppContext;
    let field = |hex: &'static str, cx: &mut gpui::App| {
        cx.new(|cx| {
            let mut field = crate::text_field::TextField::new(crate::typography::Type::Mono, hex, cx);
            field.set_text(hex, cx);
            field
        })
    };
    crate::appearance_page::AppearanceData {
        mode: wack_core::Mode::System,
        theme_colour: crate::theme::DEFAULT_THEME_COLOUR.into(),
        accent: crate::theme::DEFAULT_ACCENT.into(),
        theme_field: field(crate::theme::DEFAULT_THEME_COLOUR, cx),
        accent_field: field(crate::theme::DEFAULT_ACCENT, cx),
        backdrop,
        importing: false,
        notice: None,
    }
}
