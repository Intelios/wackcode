//! A chat's history on disk: one append-only JSON-lines file.
//!
//! Every line is written whole and flushed, so the worst a crash can leave is a cut-off last
//! line, which opening drops. History is linear: there is no branching, rewinding or editing, so
//! the file is only ever appended to. A compaction is one more line; what came before it stays in
//! the file but is no longer sent to the model.

use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use wack_llm::{Block, Message, Role, Usage};

use crate::compaction::{Summary, estimate_tokens};

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
}

pub struct Session {
    path: PathBuf,
    file: File,
    cwd: PathBuf,
    /// What the next request sends. After a compaction the first message is the summary.
    messages: Vec<Message>,
    summary: Option<Summary>,
    loaded: Vec<String>,
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
            summary: None,
            loaded: Vec::new(),
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
        let mut session =
            Self { path: path.to_path_buf(), file, cwd, messages: Vec::new(), summary: None, loaded: Vec::new(), usage: None };
        for entry in entries {
            match entry {
                Entry::Meta { .. } => {}
                Entry::Message { message, usage, .. } => session.remember(message, usage),
                Entry::Compaction { summary, keep } => session.apply_compaction(summary, keep),
                Entry::ToolsLoaded { names } => session.loaded.extend(names),
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
            .map(|(id, _, _)| Block::ToolResult { tool_use_id: id.to_string(), content: INTERRUPTED.to_string(), is_error: true })
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

    fn remember(&mut self, message: Message, usage: Option<Usage>) {
        self.messages.push(message);
        if let Some(usage) = usage {
            self.usage = Some((usage, self.messages.len()));
        }
    }

    fn apply_compaction(&mut self, summary: Summary, keep: usize) {
        let tail = self.messages.split_off(self.messages.len() - keep.min(self.messages.len()));
        self.messages = vec![Message::user(summary.render())];
        self.messages.extend(tail);
        self.summary = Some(summary);
        self.usage = None;
    }

    /// Adds a message. `usage` is the provider's count for an assistant reply.
    pub fn push(&mut self, message: Message, usage: Option<Usage>) -> Result<(), String> {
        self.append(&Entry::Message { message: message.clone(), usage, at: Some(now_millis()) })?;
        self.remember(message, usage);
        Ok(())
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
    Message { message: Message, usage: Option<Usage>, at: Option<u64> },
    /// The conversation before this point was summarised for the model.
    Compacted,
}

/// Reads a whole chat for display. Read-only: unlike [`Session::open`] it repairs nothing, so it
/// is safe while a run is writing to the same file.
pub fn history(path: &Path) -> Result<Vec<HistoryEntry>, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("Could not read the chat: {error}"))?;
    let complete = bytes.iter().rposition(|byte| *byte == b'\n').map_or(0, |index| index + 1);
    let mut entries = Vec::new();
    for line in String::from_utf8_lossy(&bytes[..complete]).lines() {
        match serde_json::from_str::<Entry>(line) {
            Ok(Entry::Message { message, usage, at }) => entries.push(HistoryEntry::Message { message, usage, at }),
            Ok(Entry::Compaction { .. }) => entries.push(HistoryEntry::Compacted),
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
    fn a_file_that_is_not_a_session_is_refused() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("notes.jsonl");
        std::fs::write(&path, "{\"hello\":1}\n").unwrap();
        assert!(Session::open(&path).is_err());
    }
}
