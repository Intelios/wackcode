//! A chat's history on disk: one append-only JSON-lines file.
//!
//! Every line is written whole and flushed, so the worst a crash can leave is a cut-off last
//! line, which opening drops. History is linear: there is no branching, rewinding or editing, so
//! the file is only ever appended to. A compaction is one more line; what came before it stays in
//! the file but is no longer sent to the model.
//!
//! Two kinds of line belong to features rather than the conversation:
//! - A **note** is a message the app wrote for the model (a mode contract). It is sent as a
//!   user-role message, but the session knows it is not the user's: the transcript hides it and a
//!   compaction never quotes it as the user's words.
//! - A **state** line is a feature's saved state (the todo list, the plan awaiting review). The
//!   latest line per key wins, and a compaction never touches it.

use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use wack_llm::{Block, Message, Role, Usage};

use crate::{
    commands::CommandUse,
    compaction::{Summary, estimate_tokens},
};

const VERSION: u32 = 1;
const INTERRUPTED: &str = "This tool was interrupted before it finished: the app quit while it ran. Check what it changed before running it again.";

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Entry {
    Meta {
        version: u32,
        cwd: PathBuf,
        created: u64,
    },
    Message {
        message: Message,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
        /// When the message was saved, in milliseconds since 1970. Display only: the transcript
        /// shows how long a turn took from it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<u64>,
        /// Set when the app wrote this message, not the user: what kind of note it is.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        /// Set when the user sent this message as a slash command: what they typed, which the
        /// transcript shows in place of the expanded text the model got.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<CommandUse>,
    },
    /// Everything before this line is replaced by `summary`, except the last `keep` messages.
    Compaction {
        summary: Summary,
        keep: usize,
    },
    /// Deferred tools the model has found, so a resumed chat offers the same tools.
    ToolsLoaded {
        names: Vec<String>,
    },
    /// A feature's saved state. The latest line for a key replaces the ones before it.
    State {
        key: String,
        value: Value,
    },
}

/// A message the app writes for the model. `tag` says what it is ("contract:plan"), so a feature
/// can tell whether the model has already been told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub tag: String,
    pub text: String,
}

pub struct Session {
    path: PathBuf,
    file: File,
    cwd: PathBuf,
    /// What the next request sends. After a compaction the first message is the summary.
    messages: Vec<Message>,
    /// One per message: the note's tag when the app wrote it.
    notes: Vec<Option<String>>,
    /// Every note tag ever written, including those since summarised away.
    noted: Vec<String>,
    summary: Option<Summary>,
    loaded: Vec<String>,
    states: Vec<(String, Value)>,
    /// The provider's count from the latest reply, and how many messages it covered.
    usage: Option<(Usage, usize)>,
}

impl Session {
    /// Starts a new session file. Fails if one already exists at `path`.
    pub fn create(path: &Path, cwd: &Path) -> Result<Self, String> {
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder).map_err(|error| format!("Could not create the sessions folder: {error}"))?;
        }
        let file = OpenOptions::new()
            .append(true)
            .create_new(true)
            .open(path)
            .map_err(|error| format!("Could not create the session file: {error}"))?;
        let mut session = Self {
            path: path.to_path_buf(),
            file,
            cwd: cwd.to_path_buf(),
            messages: Vec::new(),
            notes: Vec::new(),
            noted: Vec::new(),
            summary: None,
            loaded: Vec::new(),
            states: Vec::new(),
            usage: None,
        };
        let created = SystemTime::now().duration_since(UNIX_EPOCH).map(|time| time.as_secs()).unwrap_or(0);
        session.append(&Entry::Meta { version: VERSION, cwd: cwd.to_path_buf(), created })?;
        Ok(session)
    }

    /// Opens an existing session and repairs what a crash may have left behind.
    pub fn open(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|error| format!("Could not read the session file: {error}"))?;
        // A line that never got its newline was cut off mid-write.
        let complete = bytes.iter().rposition(|byte| *byte == b'\n').map_or(0, |index| index + 1);
        let file = OpenOptions::new()
            .append(true)
            .open(path)
            .map_err(|error| format!("Could not open the session file: {error}"))?;
        if complete < bytes.len() {
            file.set_len(complete as u64).map_err(|error| format!("Could not repair the session file: {error}"))?;
        }

        let mut entries = String::from_utf8_lossy(&bytes[..complete])
            .lines()
            .filter_map(|line| serde_json::from_str::<Entry>(line).ok())
            .collect::<Vec<_>>()
            .into_iter();
        let Some(Entry::Meta { version, cwd, .. }) = entries.next() else {
            return Err("This file is not a WackCode session.".to_string());
        };
        if version > VERSION {
            return Err("This session was written by a newer version of WackCode.".to_string());
        }
        let mut session = Self {
            path: path.to_path_buf(),
            file,
            cwd,
            messages: Vec::new(),
            notes: Vec::new(),
            noted: Vec::new(),
            summary: None,
            loaded: Vec::new(),
            states: Vec::new(),
            usage: None,
        };
        for entry in entries {
            match entry {
                Entry::Meta { .. } => {}
                Entry::Message { message, usage, note, .. } => session.remember(message, usage, note),
                Entry::Compaction { summary, keep } => session.apply_compaction(summary, keep),
                Entry::ToolsLoaded { names } => session.loaded.extend(names),
                Entry::State { key, value } => session.remember_state(key, value),
            }
        }
        session.repair()?;
        Ok(session)
    }

    /// A tool call must be followed by its result. A crash while tools ran leaves calls without
    /// one, which every provider rejects, so each gets a result saying what happened.
    fn repair(&mut self) -> Result<(), String> {
        let Some(last) = self.messages.last().filter(|message| message.role == Role::Assistant) else {
            return Ok(());
        };
        let content: Vec<Block> = last
            .tool_uses()
            .map(|(id, _, _)| Block::ToolResult { tool_use_id: id.to_string(), content: INTERRUPTED.to_string(), is_error: true, images: Vec::new() })
            .collect();
        if content.is_empty() {
            return Ok(());
        }
        self.push(Message { role: Role::User, content }, None)
    }

    fn append(&mut self, entry: &Entry) -> Result<(), String> {
        let mut line = serde_json::to_vec(entry).map_err(|error| format!("Could not save the chat: {error}"))?;
        line.push(b'\n');
        // One write per line: an append of this size lands whole or not at all.
        self.file
            .write_all(&line)
            .and_then(|()| self.file.flush())
            .map_err(|error| format!("Could not save the chat: {error}"))
    }

    fn remember(&mut self, message: Message, usage: Option<Usage>, note: Option<String>) {
        self.messages.push(message);
        if let Some(tag) = &note
            && !self.noted.contains(tag)
        {
            self.noted.push(tag.clone());
        }
        self.notes.push(note);
        if let Some(usage) = usage {
            self.usage = Some((usage, self.messages.len()));
        }
    }

    fn remember_state(&mut self, key: String, value: Value) {
        match self.states.iter_mut().find(|(known, _)| *known == key) {
            Some(state) => state.1 = value,
            None => self.states.push((key, value)),
        }
    }

    fn apply_compaction(&mut self, summary: Summary, keep: usize) {
        let from = self.messages.len() - keep.min(self.messages.len());
        let tail = self.messages.split_off(from);
        let tail_notes = self.notes.split_off(from);
        self.messages = vec![Message::user(summary.render())];
        self.messages.extend(tail);
        self.notes = vec![None];
        self.notes.extend(tail_notes);
        self.summary = Some(summary);
        self.usage = None;
    }

    /// Adds a message. `usage` is the provider's count for an assistant reply.
    pub fn push(&mut self, message: Message, usage: Option<Usage>) -> Result<(), String> {
        self.push_as(message, usage, None)
    }

    /// Adds a message the user sent as a slash command (`command`), or a plain one (`None`).
    pub fn push_as(&mut self, message: Message, usage: Option<Usage>, command: Option<CommandUse>) -> Result<(), String> {
        self.append(&Entry::Message { message: message.clone(), usage, at: Some(now_millis()), note: None, command })?;
        self.remember(message, usage, None);
        Ok(())
    }

    /// Adds a message the app wrote for the model.
    pub fn push_note(&mut self, note: Note) -> Result<(), String> {
        let message = Message::user(note.text);
        self.append(&Entry::Message { message: message.clone(), usage: None, at: Some(now_millis()), note: Some(note.tag.clone()), command: None })?;
        self.remember(message, None, Some(note.tag));
        Ok(())
    }

    /// The newest note the model can still see whose tag starts with `prefix`.
    pub fn latest_note(&self, prefix: &str) -> Option<&str> {
        self.notes.iter().rev().flatten().map(String::as_str).find(|tag| tag.starts_with(prefix))
    }

    /// Whether a note with this prefix was ever written, even if since summarised away.
    pub fn ever_noted(&self, prefix: &str) -> bool {
        self.noted.iter().any(|tag| tag.starts_with(prefix))
    }

    /// Whether message `index` is the app's note rather than part of the conversation.
    pub fn is_note(&self, index: usize) -> bool {
        self.notes.get(index).is_some_and(Option::is_some)
    }

    /// A feature's saved state.
    pub fn state(&self, key: &str) -> Option<&Value> {
        self.states.iter().find(|(known, _)| known == key).map(|(_, value)| value)
    }

    /// Saves a feature's state. Returns false, writing nothing, when it has not changed.
    pub fn set_state(&mut self, key: &str, value: Value) -> Result<bool, String> {
        if self.state(key) == Some(&value) {
            return Ok(false);
        }
        self.append(&Entry::State { key: key.to_string(), value: value.clone() })?;
        self.remember_state(key.to_string(), value);
        Ok(true)
    }

    /// Replaces all but the last `keep` messages with a summary.
    pub fn compact(&mut self, summary: Summary, keep: usize) -> Result<(), String> {
        self.append(&Entry::Compaction { summary: summary.clone(), keep })?;
        self.apply_compaction(summary, keep);
        Ok(())
    }

    /// Records a deferred tool as found. Returns false if it already was.
    pub fn load_tool(&mut self, name: &str) -> Result<bool, String> {
        if self.loaded.iter().any(|loaded| loaded == name) {
            return Ok(false);
        }
        self.append(&Entry::ToolsLoaded { names: vec![name.to_string()] })?;
        self.loaded.push(name.to_string());
        Ok(true)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn summary(&self) -> Option<&Summary> {
        self.summary.as_ref()
    }

    pub fn loaded_tools(&self) -> &[String] {
        &self.loaded
    }

    /// How much of the model's context the next request will use: the provider's last count plus
    /// an estimate of what was added since.
    pub fn context_tokens(&self) -> u64 {
        match self.usage {
            Some((usage, covered)) => {
                usage.context_tokens() + usage.output + estimate_tokens(&self.messages[covered.min(self.messages.len())..])
            }
            None => estimate_tokens(&self.messages),
        }
    }
}

fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|time| time.as_millis() as u64).unwrap_or(0)
}

/// One step of a chat as the user saw it, for drawing the transcript. Unlike
/// [`Session::messages`], nothing is dropped at a compaction.
#[derive(Clone, Debug, PartialEq)]
pub enum HistoryEntry {
    Message { message: Message, usage: Option<Usage>, at: Option<u64>, command: Option<CommandUse> },
    /// The conversation before this point was summarised for the model.
    Compacted,
    /// The app told the model something here (a mode contract, a background task's news). Not
    /// shown as a message.
    Note { tag: String, text: String, at: Option<u64> },
    /// A feature saved its state here.
    State { key: String, value: Value },
}

/// Reads a whole chat for display. Read-only: unlike [`Session::open`] it repairs nothing, so it
/// is safe while a run is writing to the same file.
pub fn history(path: &Path) -> Result<Vec<HistoryEntry>, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("Could not read the chat: {error}"))?;
    let complete = bytes.iter().rposition(|byte| *byte == b'\n').map_or(0, |index| index + 1);
    let mut entries = Vec::new();
    for line in String::from_utf8_lossy(&bytes[..complete]).lines() {
        match serde_json::from_str::<Entry>(line) {
            Ok(Entry::Message { note: Some(tag), message, at, .. }) => entries.push(HistoryEntry::Note { tag, text: message.text(), at }),
            Ok(Entry::Message { message, usage, at, note: None, command }) => entries.push(HistoryEntry::Message { message, usage, at, command }),
            Ok(Entry::Compaction { .. }) => entries.push(HistoryEntry::Compacted),
            Ok(Entry::State { key, value }) => entries.push(HistoryEntry::State { key, value }),
            Ok(Entry::Meta { .. } | Entry::ToolsLoaded { .. }) | Err(_) => {}
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn tool_call() -> Message {
        Message {
            role: Role::Assistant,
            content: vec![Block::ToolUse { id: "t1".into(), name: "bash".into(), input: json!({ "command": "make" }) }],
        }
    }

    #[test]
    fn reopening_restores_messages_tools_and_usage() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("sessions/chat.jsonl");
        let mut session = Session::create(&path, folder.path()).unwrap();
        session.push(Message::user("hello"), None).unwrap();
        let usage = Usage { input: 10, output: 5, cache_read: 100, cache_write: 0 };
        session.push(Message { role: Role::Assistant, content: vec![Block::Text { text: "hi".into() }] }, Some(usage)).unwrap();
        assert!(session.load_tool("browser").unwrap());
        assert!(!session.load_tool("browser").unwrap());
        drop(session);

        let session = Session::open(&path).unwrap();
        assert_eq!(session.messages().len(), 2);
        assert_eq!(session.loaded_tools(), ["browser"]);
        assert_eq!(session.context_tokens(), 115);
        assert_eq!(session.cwd(), folder.path());
        assert!(Session::create(&path, folder.path()).is_err(), "an existing session is never overwritten");
    }

    #[test]
    fn a_crash_mid_write_and_mid_tool_is_repaired() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("chat.jsonl");
        let mut session = Session::create(&path, folder.path()).unwrap();
        session.push(Message::user("build it"), None).unwrap();
        session.push(tool_call(), None).unwrap();
        drop(session);
        // The process died while writing the next line.
        OpenOptions::new().append(true).open(&path).unwrap().write_all(b"{\"type\":\"message\",\"mess").unwrap();

        let mut session = Session::open(&path).unwrap();
        let last = session.messages().last().unwrap();
        assert!(matches!(&last.content[0], Block::ToolResult { tool_use_id, is_error: true, .. } if tool_use_id == "t1"));
        session.push(Message::user("carry on"), None).unwrap();
        drop(session);

        // The repair was saved and the cut-off line is gone, so every line parses again.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.lines().all(|line| serde_json::from_str::<Entry>(line).is_ok()));
        assert_eq!(Session::open(&path).unwrap().messages().len(), 4);
    }

    #[test]
    fn a_compaction_survives_reopening() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("chat.jsonl");
        let mut session = Session::create(&path, folder.path()).unwrap();
        for text in ["one", "two", "three"] {
            session.push(Message::user(text), None).unwrap();
        }
        let summary = Summary { body: "The story so far".into(), ..Summary::default() };
        session.compact(summary.clone(), 1).unwrap();
        session.push(Message::user("four"), None).unwrap();

        for session in [session, Session::open(&path).unwrap()] {
            let texts: Vec<String> = session.messages().iter().map(Message::text).collect();
            assert!(texts[0].contains("The story so far"));
            assert_eq!(texts[1..], ["three", "four"]);
            assert_eq!(session.summary(), Some(&summary));
        }
    }

    #[test]
    fn history_keeps_everything_with_times() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("chat.jsonl");
        let mut session = Session::create(&path, folder.path()).unwrap();
        session.push(Message::user("one"), None).unwrap();
        session.compact(Summary { body: "so far".into(), ..Summary::default() }, 0).unwrap();
        session.push(Message::user("two"), None).unwrap();
        // A line from before messages carried a time still reads.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"old\"}]}}\n")
            .unwrap();

        let entries = history(&path).unwrap();
        assert_eq!(entries.len(), 4);
        assert!(matches!(&entries[0], HistoryEntry::Message { message, at: Some(_), .. } if message.text() == "one"));
        assert_eq!(entries[1], HistoryEntry::Compacted);
        assert!(matches!(&entries[3], HistoryEntry::Message { at: None, .. }));
        assert_eq!(Session::open(&path).unwrap().messages().len(), 3);
    }

    #[test]
    fn notes_and_state_survive_reopening_and_compaction() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("chat.jsonl");
        let mut session = Session::create(&path, folder.path()).unwrap();
        session.push_note(Note { tag: "contract:plan".into(), text: "Plan rules".into() }).unwrap();
        session.push(Message::user("one"), None).unwrap();
        assert!(session.set_state("todo", json!({ "tasks": [1] })).unwrap());
        assert!(!session.set_state("todo", json!({ "tasks": [1] })).unwrap(), "an unchanged state writes nothing");
        assert!(session.set_state("todo", json!({ "tasks": [1, 2] })).unwrap());
        assert!(session.is_note(0) && !session.is_note(1));
        assert_eq!(session.latest_note("contract:"), Some("contract:plan"));

        // The note is summarised away; the state is not.
        session.compact(Summary { body: "so far".into(), ..Summary::default() }, 1).unwrap();
        session.push_note(Note { tag: "contract:normal".into(), text: "Normal rules".into() }).unwrap();
        for session in [session, Session::open(&path).unwrap()] {
            assert_eq!(session.latest_note("contract:"), Some("contract:normal"));
            assert!(session.ever_noted("contract:plan"));
            assert_eq!(session.state("todo"), Some(&json!({ "tasks": [1, 2] })));
            assert!(!session.is_note(0) && !session.is_note(1) && session.is_note(2));
            assert_eq!(session.messages()[2].text(), "Normal rules");
        }

        let kinds: Vec<String> = history(&path)
            .unwrap()
            .iter()
            .map(|entry| match entry {
                HistoryEntry::Message { message, .. } => format!("message {}", message.text()),
                HistoryEntry::Compacted => "compacted".to_string(),
                HistoryEntry::Note { tag, .. } => format!("note {tag}"),
                HistoryEntry::State { key, .. } => format!("state {key}"),
            })
            .collect();
        assert_eq!(kinds, ["note contract:plan", "message one", "state todo", "state todo", "compacted", "note contract:normal"]);
    }

    #[test]
    fn tool_result_pictures_round_trip_and_older_lines_still_load() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("chat.jsonl");
        let mut session = Session::create(&path, folder.path()).unwrap();
        session.push(tool_call(), None).unwrap();
        let image = wack_llm::Image::new("image/png", "AAAA");
        let result = Message {
            role: Role::User,
            content: vec![Block::ToolResult { tool_use_id: "t1".into(), content: "Shot.".into(), is_error: false, images: vec![image] }],
        };
        session.push(result.clone(), None).unwrap();
        // A result without pictures is written exactly as before the field existed.
        session.push(Message { role: Role::User, content: vec![Block::Text { text: "next".into() }] }, None).unwrap();
        let file = std::fs::read_to_string(&path).unwrap();
        assert_eq!(file.matches("\"images\"").count(), 1);
        assert_eq!(Session::open(&path).unwrap().messages()[1], result);

        let old = r#"{"type":"tool_result","tool_use_id":"t1","content":"ok","is_error":false}"#;
        let block: Block = serde_json::from_str(old).unwrap();
        assert_eq!(block, Block::ToolResult { tool_use_id: "t1".into(), content: "ok".into(), is_error: false, images: Vec::new() });
        assert_eq!(serde_json::to_string(&block).unwrap(), old);
    }

    #[test]
    fn a_file_that_is_not_a_session_is_refused() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("notes.jsonl");
        std::fs::write(&path, "{\"hello\":1}\n").unwrap();
        assert!(Session::open(&path).is_err());
    }
}
