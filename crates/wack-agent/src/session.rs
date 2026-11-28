//! A chat's history on disk: one append-only JSON-lines file.
//!
//! Every line is written whole and flushed, so the worst a crash can leave is a cut-off last
//! line, which opening drops. The file is only ever appended to. A compaction is one more line;
//! what came before it stays in the file but is no longer sent to the model.
//!
//! The user can take the conversation back to just before one of their messages (rewinding,
//! which editing and retrying a message also do). That is one more line too, saying how much of
//! the conversation still stands ([`rewind`]): reading the file replays every line and drops what
//! a rewind took away, so a rewound conversation reads exactly as it did before that message.
//! Until anything else is written, the rewind can be undone ([`unrewind`]). There are no
//! branches: what a rewind took away is gone once the conversation carries on.
//!
//! The user's messages are found by counting ([`is_prompt`]): the transcript counts the same
//! messages the same way, so "the third message" means one message to both.
//!
//! Before each message the user sends, a run notes where the chat's files stood
//! ([`Session::checkpoint`], a tree in `wack_core::checkpoints`' store), so a rewind can put the
//! files back too.
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
        /// A reply that reasoned: how long the model thought, from the request until its first
        /// words or call, in milliseconds. Display only ("Thought for 12s").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thought_ms: Option<u64>,
    },
    /// Everything before this line is replaced by `summary`, except the last `keep` messages.
    Compaction {
        summary: Summary,
        keep: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tokens_before: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tokens_after: Option<u64>,
        /// What the call that wrote the summary used, when the provider said.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
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
    /// The chat's files as they stood before the user's next message: a tree in the chat's
    /// checkpoint store.
    Checkpoint {
        tree: String,
    },
    /// The user took the conversation back: of the lines that stood, only the first `keep`
    /// still do. `files` puts back the files the rewind restored, when it restored any.
    Rewind {
        keep: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        files: Option<FilesUndo>,
    },
    /// Undoes the rewind on the line before it.
    Unrewind,
}

/// How to undo the files a rewind put back: the tree they stood at just before, and which of
/// them it changed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesUndo {
    pub tree: String,
    pub paths: Vec<String>,
}

/// The lines that still stand after every rewind, in order, without the meta line; and, when
/// the file's last line is a rewind, what it took away and how to put its files back.
struct Standing {
    entries: Vec<Entry>,
    last_rewind: Option<(Vec<Entry>, Option<FilesUndo>)>,
}

fn standing(lines: impl IntoIterator<Item = Entry>) -> Standing {
    let mut entries = Vec::new();
    let mut last_rewind = None;
    for line in lines {
        match line {
            Entry::Meta { .. } => {}
            Entry::Rewind { keep, files } => {
                let gone = entries.split_off(keep.min(entries.len()));
                last_rewind = Some((gone, files));
            }
            Entry::Unrewind => {
                if let Some((gone, _)) = last_rewind.take() {
                    entries.extend(gone);
                }
            }
            line => {
                last_rewind = None;
                entries.push(line);
            }
        }
    }
    Standing { entries, last_rewind }
}

/// Whether a line is one of the user's messages, as the transcript counts them: a user message
/// the app did not write, holding no tool results or, beside them, words sent while the agent
/// worked.
fn is_prompt(entry: &Entry) -> bool {
    let Entry::Message { message, note: None, .. } = entry else {
        return false;
    };
    if message.role != Role::User {
        return false;
    }
    let results = message.content.iter().any(|block| matches!(block, Block::ToolResult { .. }));
    !results || !message.text().trim().is_empty()
}

/// Where the user's `prompt`-th message (from 0) stands among the lines that stand.
fn prompt_position(entries: &[Entry], prompt: usize) -> Result<usize, String> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| is_prompt(entry))
        .nth(prompt)
        .map(|(index, _)| index)
        .ok_or_else(|| "That message is no longer in the chat.".to_string())
}

/// Every complete line of a chat's file, parsed, and how many bytes they take.
fn read_lines(path: &Path) -> Result<(Vec<Entry>, usize), String> {
    let bytes = std::fs::read(path).map_err(|error| format!("Could not read the chat: {error}"))?;
    // A line that never got its newline was cut off mid-write.
    let complete = bytes.iter().rposition(|byte| *byte == b'\n').map_or(0, |index| index + 1);
    let entries = String::from_utf8_lossy(&bytes[..complete])
        .lines()
        .filter_map(|line| serde_json::from_str::<Entry>(line).ok())
        .collect();
    Ok((entries, complete))
}

/// Appends one line to a chat's file outside a run, dropping a cut-off last line first.
fn append_line(path: &Path, complete: usize, entry: &Entry) -> Result<(), String> {
    let mut file = OpenOptions::new().append(true).open(path).map_err(|error| format!("Could not open the chat: {error}"))?;
    file.set_len(complete as u64).map_err(|error| format!("Could not repair the chat: {error}"))?;
    let mut line = serde_json::to_vec(entry).map_err(|error| format!("Could not save the chat: {error}"))?;
    line.push(b'\n');
    file.write_all(&line).and_then(|()| file.flush()).map_err(|error| format!("Could not save the chat: {error}"))
}

/// The checkpoint taken just before the user's `prompt`-th message (from 0): the tree of the
/// chat's files then. `None` when there is none: the message was sent while the agent worked,
/// or no snapshot could be taken.
pub fn prompt_checkpoint(path: &Path, prompt: usize) -> Result<Option<String>, String> {
    let (lines, _) = read_lines(path)?;
    let entries = standing(lines).entries;
    let position = prompt_position(&entries, prompt)?;
    for entry in entries[..position].iter().rev() {
        match entry {
            Entry::Checkpoint { tree } => return Ok(Some(tree.clone())),
            // What the run wrote before the message: its state and notes.
            Entry::State { .. } | Entry::ToolsLoaded { .. } | Entry::Message { note: Some(_), .. } => {}
            _ => return Ok(None),
        }
    }
    Ok(None)
}

/// Takes a chat back to just before the user's `prompt`-th message (from 0). What the app told
/// the model before it (its notes) stays. `files` undoes the files put back with it, if any.
/// Call it only while no run holds the file.
pub fn rewind(path: &Path, prompt: usize, files: Option<FilesUndo>) -> Result<(), String> {
    let (lines, complete) = read_lines(path)?;
    let entries = standing(lines).entries;
    let keep = prompt_position(&entries, prompt)?;
    append_line(path, complete, &Entry::Rewind { keep, files })
}

/// Undoes the chat's last rewind, while it is still the file's last line, and returns how to put
/// back the files it restored. Call it only while no run holds the file.
pub fn unrewind(path: &Path) -> Result<Option<FilesUndo>, String> {
    let (lines, complete) = read_lines(path)?;
    let Some((_, files)) = standing(lines).last_rewind else {
        return Err("The chat has moved on since the rewind, so it can't be undone.".to_string());
    };
    append_line(path, complete, &Entry::Unrewind)?;
    Ok(files)
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

        let mut lines = String::from_utf8_lossy(&bytes[..complete])
            .lines()
            .filter_map(|line| serde_json::from_str::<Entry>(line).ok())
            .collect::<Vec<_>>()
            .into_iter();
        let Some(Entry::Meta { version, cwd, .. }) = lines.next() else {
            return Err("This file is not a WackCode session.".to_string());
        };
        let entries = standing(lines).entries;
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
                Entry::Compaction { summary, keep, .. } => session.apply_compaction(summary, keep),
                Entry::ToolsLoaded { names } => session.loaded.extend(names),
                Entry::State { key, value } => session.remember_state(key, value),
                Entry::Checkpoint { .. } | Entry::Rewind { .. } | Entry::Unrewind => {}
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
        self.append(&Entry::Message { message: message.clone(), usage, at: Some(now_millis()), note: None, command, thought_ms: None })?;
        self.remember(message, usage, None);
        Ok(())
    }

    /// Adds the model's reply, with how long it thought when it did.
    pub fn push_reply(&mut self, message: Message, usage: Option<Usage>, thought_ms: Option<u64>) -> Result<(), String> {
        self.append(&Entry::Message { message: message.clone(), usage, at: Some(now_millis()), note: None, command: None, thought_ms })?;
        self.remember(message, usage, None);
        Ok(())
    }

    /// Adds a message the app wrote for the model.
    pub fn push_note(&mut self, note: Note) -> Result<(), String> {
        let message = Message::user(note.text);
        self.append(&Entry::Message { message: message.clone(), usage: None, at: Some(now_millis()), note: Some(note.tag.clone()), command: None, thought_ms: None })?;
        self.remember(message, None, Some(note.tag));
        Ok(())
    }

    /// The newest note the model can still see whose tag starts with `prefix`.
    pub fn latest_note(&self, prefix: &str) -> Option<&str> {
        self.latest_note_with_text(prefix).map(|(tag, _)| tag)
    }

    /// The newest note the model can still see whose tag starts with `prefix`, with its text.
    pub fn latest_note_with_text(&self, prefix: &str) -> Option<(&str, &str)> {
        self.notes
            .iter()
            .zip(&self.messages)
            .rev()
            .find_map(|(note, message)| {
                let tag = note.as_deref().filter(|tag| tag.starts_with(prefix))?;
                let text = match message.content.first() {
                    Some(Block::Text { text }) => text.as_str(),
                    _ => "",
                };
                Some((tag, text))
            })
    }

    /// Whether a note with this prefix was ever written, even if since summarised away.
    pub fn ever_noted(&self, prefix: &str) -> bool {
        self.noted.iter().any(|tag| tag.starts_with(prefix))
    }

    /// The text of the user's latest message: the newest user message that is neither the app's
    /// note nor tool results.
    pub fn latest_request(&self) -> Option<&str> {
        self.messages.iter().zip(&self.notes).rev().find_map(|(message, note)| {
            if message.role != Role::User || note.is_some() || message.content.iter().any(|block| matches!(block, Block::ToolResult { .. })) {
                return None;
            }
            message.content.iter().find_map(|block| match block {
                Block::Text { text } if !text.trim().is_empty() => Some(text.as_str()),
                _ => None,
            })
        })
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

    /// Replaces all but the last `keep` messages with a summary, which a call that used `usage`
    /// wrote. Returns `(tokens_before, tokens_after)`: the context size before compaction and the
    /// estimated size after.
    pub fn compact(&mut self, summary: Summary, keep: usize, usage: Option<Usage>) -> Result<(u64, u64), String> {
        let tokens_before = self.context_tokens();
        let from = self.messages.len() - keep.min(self.messages.len());
        let mut after = Vec::with_capacity(1 + self.messages.len() - from);
        after.push(Message::user(summary.render()));
        after.extend_from_slice(&self.messages[from..]);
        let tokens_after = estimate_tokens(&after);
        self.append(&Entry::Compaction {
            summary: summary.clone(),
            keep,
            tokens_before: Some(tokens_before),
            tokens_after: Some(tokens_after),
            usage,
        })?;
        self.apply_compaction(summary, keep);
        Ok((tokens_before, tokens_after))
    }

    /// Notes where the chat's files stand before the user's next message: `tree`, a snapshot in
    /// the chat's checkpoint store.
    pub fn checkpoint(&mut self, tree: &str) -> Result<(), String> {
        self.append(&Entry::Checkpoint { tree: tree.to_string() })
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
    Message { message: Message, usage: Option<Usage>, at: Option<u64>, command: Option<CommandUse>, thought_ms: Option<u64> },
    /// The conversation before this point was summarised for the model.
    Compacted {
        summary: Summary,
        tokens_before: Option<u64>,
        tokens_after: Option<u64>,
    },
    /// The app told the model something here (a mode contract, a background task's news). Not
    /// shown as a message.
    Note { tag: String, text: String, at: Option<u64> },
    /// A feature saved its state here.
    State { key: String, value: Value },
    /// Always last: the user has just rewound the conversation, and nothing has been written
    /// since, so it can be undone ([`unrewind`]). `files`: the rewind put files back too.
    Rewound { files: bool },
}

/// Reads a whole chat for display, as it stands after any rewinds. Read-only: unlike
/// [`Session::open`] it repairs nothing, so it is safe while a run is writing to the same file.
pub fn history(path: &Path) -> Result<Vec<HistoryEntry>, String> {
    history_with_spend(path).map(|(entries, _)| entries)
}

/// [`history`], and every token the chat's own model calls used, from the same read: each reply
/// and each compaction's summary the file records, those a rewind took away included, since
/// they were spent all the same. Its sub-agents' calls are in their own files.
pub fn history_with_spend(path: &Path) -> Result<(Vec<HistoryEntry>, Usage), String> {
    let (lines, _) = read_lines(path)?;
    let mut spend = Usage::default();
    for line in &lines {
        if let Entry::Message { usage: Some(usage), .. } | Entry::Compaction { usage: Some(usage), .. } = line {
            spend += *usage;
        }
    }
    let Standing { entries: lines, last_rewind } = standing(lines);
    let mut entries = Vec::new();
    for line in lines {
        match line {
            Entry::Message { note: Some(tag), message, at, .. } => entries.push(HistoryEntry::Note { tag, text: message.text(), at }),
            Entry::Message { message, usage, at, note: None, command, thought_ms } => {
                entries.push(HistoryEntry::Message { message, usage, at, command, thought_ms })
            }
            Entry::Compaction { summary, tokens_before, tokens_after, .. } => {
                entries.push(HistoryEntry::Compacted { summary, tokens_before, tokens_after });
            }
            Entry::State { key, value } => entries.push(HistoryEntry::State { key, value }),
            Entry::Meta { .. } | Entry::ToolsLoaded { .. } | Entry::Checkpoint { .. } | Entry::Rewind { .. } | Entry::Unrewind => {}
        }
    }
    if let Some((_, files)) = last_rewind {
        entries.push(HistoryEntry::Rewound { files: files.is_some() });
    }
    Ok((entries, spend))
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
        session.compact(summary.clone(), 1, None).unwrap();
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
        session.compact(Summary { body: "so far".into(), ..Summary::default() }, 0, None).unwrap();
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
        assert!(matches!(
            &entries[1],
            HistoryEntry::Compacted { summary, tokens_before: Some(_), tokens_after: Some(_) } if summary.body == "so far"
        ));
        assert!(matches!(&entries[3], HistoryEntry::Message { at: None, .. }));
        assert_eq!(Session::open(&path).unwrap().messages().len(), 3);

        // An older compaction line without saved token counts still loads with its summary.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"type\":\"compaction\",\"summary\":{\"body\":\"legacy body\",\"user_messages\":[],\"omitted_user_messages\":0,\"read_files\":[],\"modified_files\":[]},\"keep\":0}\n")
            .unwrap();
        let entries = history(&path).unwrap();
        assert!(matches!(
            entries.last(),
            Some(HistoryEntry::Compacted { summary, tokens_before: None, tokens_after: None }) if summary.body == "legacy body"
        ));
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
        session.compact(Summary { body: "so far".into(), ..Summary::default() }, 1, None).unwrap();
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
                HistoryEntry::Compacted { .. } => "compacted".to_string(),
                HistoryEntry::Note { tag, .. } => format!("note {tag}"),
                HistoryEntry::State { key, .. } => format!("state {key}"),
                HistoryEntry::Rewound { .. } => "rewound".to_string(),
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

    fn texts(path: &Path) -> Vec<String> {
        Session::open(path).unwrap().messages().iter().map(Message::text).collect()
    }

    fn reply(text: &str) -> Message {
        Message { role: Role::Assistant, content: vec![Block::Text { text: text.into() }] }
    }

    #[test]
    fn a_rewind_takes_the_chat_back_to_before_a_message_and_can_be_undone() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("chat.jsonl");
        let mut session = Session::create(&path, folder.path()).unwrap();
        session.checkpoint("tree-one").unwrap();
        session.push(Message::user("one"), None).unwrap();
        session.push(reply("first"), None).unwrap();
        session.checkpoint("tree-two").unwrap();
        session.push_note(Note { tag: "contract:plan".into(), text: "Plan rules".into() }).unwrap();
        assert!(session.set_state("todo", json!({ "tasks": [2] })).unwrap());
        session.push(Message::user("two"), None).unwrap();
        session.push(tool_call(), None).unwrap();
        let result = Message { role: Role::User, content: vec![Block::ToolResult { tool_use_id: "t1".into(), content: "ok".into(), is_error: false, images: Vec::new() }] };
        session.push(result, None).unwrap();
        // Words sent while the agent worked count as a message of the user's.
        session.push(Message::user("steered"), None).unwrap();
        session.push(reply("second"), None).unwrap();
        drop(session);

        assert_eq!(prompt_checkpoint(&path, 0).unwrap().as_deref(), Some("tree-one"));
        assert_eq!(prompt_checkpoint(&path, 1).unwrap().as_deref(), Some("tree-two"), "past the run's note and state");
        assert_eq!(prompt_checkpoint(&path, 2).unwrap(), None, "steered in mid-run: no snapshot");
        assert!(prompt_checkpoint(&path, 3).is_err());

        let undo = FilesUndo { tree: "before-rewind".into(), paths: vec!["a.txt".into()] };
        rewind(&path, 1, Some(undo.clone())).unwrap();
        // What the app told the model before the message stays; the message and after go.
        assert_eq!(texts(&path), ["one", "first", "Plan rules"]);
        assert!(matches!(history(&path).unwrap().last(), Some(HistoryEntry::Rewound { files: true })));
        assert_eq!(Session::open(&path).unwrap().state("todo"), Some(&json!({ "tasks": [2] })));

        assert_eq!(unrewind(&path).unwrap(), Some(undo));
        assert_eq!(texts(&path).len(), 8);
        assert!(!history(&path).unwrap().iter().any(|entry| matches!(entry, HistoryEntry::Rewound { .. })));
        assert!(unrewind(&path).is_err(), "an undone rewind can't be undone again");

        // Rewound to the first message, then the chat carries on: the rewind is final.
        rewind(&path, 0, None).unwrap();
        assert!(texts(&path).is_empty());
        let mut session = Session::open(&path).unwrap();
        session.push(Message::user("again"), None).unwrap();
        drop(session);
        assert_eq!(texts(&path), ["again"]);
        assert!(unrewind(&path).is_err());
        assert!(!history(&path).unwrap().iter().any(|entry| matches!(entry, HistoryEntry::Rewound { .. })));
    }

    #[test]
    fn the_spend_counts_every_call_a_rewind_or_compaction_hid() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("chat.jsonl");
        let used = |input, output| Some(Usage { input, output, cache_read: 1_000, cache_write: 10 });
        let mut session = Session::create(&path, folder.path()).unwrap();
        session.push(Message::user("one"), None).unwrap();
        session.push(reply("first"), used(100, 20)).unwrap();
        session.compact(Summary { body: "so far".into(), ..Summary::default() }, 0, used(300, 40)).unwrap();
        session.push(Message::user("two"), None).unwrap();
        session.push(reply("second"), used(200, 30)).unwrap();
        drop(session);
        let all = Usage { input: 600, output: 90, cache_read: 3_000, cache_write: 30 };
        assert_eq!(history_with_spend(&path).unwrap().1, all);

        // The second reply leaves the conversation, but its tokens were spent all the same.
        rewind(&path, 1, None).unwrap();
        let (history, spend) = history_with_spend(&path).unwrap();
        assert!(!history.iter().any(|entry| matches!(entry, HistoryEntry::Message { message, .. } if message.text() == "second")));
        assert_eq!(spend, all);
    }

    #[test]
    fn a_rewind_past_a_compaction_brings_the_whole_conversation_back() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("chat.jsonl");
        let mut session = Session::create(&path, folder.path()).unwrap();
        for text in ["one", "two", "three"] {
            session.push(Message::user(text), None).unwrap();
        }
        session.compact(Summary { body: "so far".into(), ..Summary::default() }, 1, None).unwrap();
        session.push(Message::user("four"), None).unwrap();
        drop(session);
        assert!(texts(&path)[0].contains("so far"));
        // The rewind counts messages as the user saw them, compaction or not.
        rewind(&path, 3, None).unwrap();
        assert!(texts(&path)[0].contains("so far"), "before \"four\" the summary still stands");
        rewind(&path, 1, None).unwrap();
        assert_eq!(texts(&path), ["one"]);
        assert_eq!(Session::open(&path).unwrap().summary(), None);
    }

    #[test]
    fn a_rewind_after_a_cut_off_line_still_reads() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("chat.jsonl");
        let mut session = Session::create(&path, folder.path()).unwrap();
        session.push(Message::user("one"), None).unwrap();
        session.push(reply("hi"), None).unwrap();
        drop(session);
        OpenOptions::new().append(true).open(&path).unwrap().write_all(b"{\"type\":\"mess").unwrap();
        rewind(&path, 0, None).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.lines().all(|line| serde_json::from_str::<Entry>(line).is_ok()));
        assert!(texts(&path).is_empty());
    }

    #[test]
    fn a_file_that_is_not_a_session_is_refused() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("notes.jsonl");
        std::fs::write(&path, "{\"hello\":1}\n").unwrap();
        assert!(Session::open(&path).is_err());
    }
}
