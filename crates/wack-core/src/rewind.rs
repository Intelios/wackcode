//! Taking a chat back to just before one of the user's messages: what rewinding does, and what
//! editing and retrying a message do before they send it again.
//!
//! The conversation goes back by one line in the chat's file (`wack_agent::session::rewind`);
//! the files go back only when the user chooses, from the snapshot taken before that message
//! (`checkpoints`), and only the files they ticked. A rewind that put files back remembers how
//! to undo them, so undoing the rewind puts both back.
//!
//! Every call is blocking (it reads the chat's file and runs Git): call it off the UI thread,
//! and only while no run holds the chat's file.

use std::path::Path;

use wack_agent::FilesUndo;

use crate::checkpoints::{Change, Checkpoints};

/// What rewinding to before a message would find.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preview {
    /// The snapshot taken just before the message, when there is one: none for a message sent
    /// while the agent worked, or where no snapshot could be taken.
    pub checkpoint: Option<String>,
    /// The files that changed since it, which the rewind can put back.
    pub changes: Vec<Change>,
    /// Why the files can't be compared with the snapshot, when they can't.
    pub problem: Option<String>,
}

/// The files a rewind, or undoing one, put back and left alone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Files {
    pub restored: Vec<String>,
    pub skipped: Vec<String>,
}

/// What rewinding the chat in `session` to before the user's `prompt`-th message (from 0) would
/// find in `cwd`.
pub fn preview(session: &Path, prompt: usize, store: &Checkpoints, cwd: &Path) -> Result<Preview, String> {
    let Some(checkpoint) = wack_agent::prompt_checkpoint(session, prompt)? else {
        return Ok(Preview::default());
    };
    Ok(match store.changes(cwd, &checkpoint) {
        Ok(changes) => Preview { checkpoint: Some(checkpoint), changes, problem: None },
        Err(problem) => Preview { checkpoint: Some(checkpoint), changes: Vec::new(), problem: Some(problem) },
    })
}

/// Files to put back with a rewind: `paths` of them, as they stood at `checkpoint`.
pub struct Restore<'a> {
    pub store: &'a Checkpoints,
    pub cwd: &'a Path,
    pub checkpoint: &'a str,
    pub paths: &'a [String],
}

/// Takes the chat in `session` back to just before the user's `prompt`-th message (from 0),
/// putting back the files `restore` names first. When the conversation can't go back, the files
/// are put back as they were and the error says why.
pub fn rewind(session: &Path, prompt: usize, restore: Option<Restore>) -> Result<Files, String> {
    let Some(restore) = restore.filter(|restore| !restore.paths.is_empty()) else {
        wack_agent::rewind(session, prompt, None)?;
        return Ok(Files::default());
    };
    let restored = restore.store.restore(restore.cwd, restore.checkpoint, Some(restore.paths))?;
    let undo = (!restored.restored.is_empty()).then(|| FilesUndo { tree: restored.undo.clone(), paths: restored.restored.clone() });
    if let Err(message) = wack_agent::rewind(session, prompt, undo) {
        let _ = restore.store.restore(restore.cwd, &restored.undo, Some(&restored.restored));
        return Err(message);
    }
    Ok(Files { restored: restored.restored, skipped: restored.skipped })
}

/// Undoes the chat's last rewind while nothing has been written since: the conversation comes
/// back, and so do the files it put back.
pub fn undo(session: &Path, store: &Checkpoints, cwd: &Path) -> Result<Files, String> {
    let Some(files) = wack_agent::unrewind(session)? else {
        return Ok(Files::default());
    };
    let restored = store.restore(cwd, &files.tree, Some(&files.paths))?;
    Ok(Files { restored: restored.restored, skipped: restored.skipped })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use wack_agent::{Session, history};
    use wack_llm::{Block, Message, Role};

    use super::*;
    use crate::checkpoints::Action;

    fn reply(text: &str) -> Message {
        Message { role: Role::Assistant, content: vec![Block::Text { text: text.into() }] }
    }

    #[test]
    fn a_rewind_puts_back_the_files_ticked_and_its_undo_brings_them_back() {
        let folder = tempfile::tempdir().unwrap();
        let project = folder.path().join("project");
        let data = folder.path().join("data");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&data).unwrap();
        let store = Checkpoints::new(data.join("v3/checkpoints/chat"), data.clone());
        let path = data.join("v3/sessions/chat.jsonl");

        fs::write(project.join("app.rs"), "fn main() {}\n").unwrap();
        fs::write(project.join("notes.md"), "mine\n").unwrap();
        let mut session = Session::create(&path, &project).unwrap();
        session.checkpoint(&store.snapshot(&project).unwrap()).unwrap();
        session.push(Message::user("add a feature"), None).unwrap();
        session.push(reply("done"), None).unwrap();
        drop(session);
        // The agent changed one file and added another; the user changed their notes.
        fs::write(project.join("app.rs"), "fn main() { feature(); }\n").unwrap();
        fs::write(project.join("feature.rs"), "fn feature() {}\n").unwrap();
        fs::write(project.join("notes.md"), "mine, edited\n").unwrap();

        let preview = preview(&path, 0, &store, &project).unwrap();
        let mut changes: Vec<(String, Action)> = preview.changes.iter().map(|change| (change.path.clone(), change.action)).collect();
        changes.sort();
        assert_eq!(changes, [("app.rs".into(), Action::Revert), ("feature.rs".into(), Action::Delete), ("notes.md".into(), Action::Revert)]);

        // The user keeps their notes.
        let ticked = vec!["app.rs".to_string(), "feature.rs".to_string()];
        let checkpoint = preview.checkpoint.unwrap();
        let files = rewind(&path, 0, Some(Restore { store: &store, cwd: &project, checkpoint: &checkpoint, paths: &ticked })).unwrap();
        assert_eq!(files.restored, ticked);
        assert_eq!(fs::read_to_string(project.join("app.rs")).unwrap(), "fn main() {}\n");
        assert!(!project.join("feature.rs").exists());
        assert_eq!(fs::read_to_string(project.join("notes.md")).unwrap(), "mine, edited\n");
        assert!(matches!(history(&path).unwrap().last(), Some(wack_agent::HistoryEntry::Rewound { files: true })));

        undo(&path, &store, &project).unwrap();
        assert_eq!(fs::read_to_string(project.join("app.rs")).unwrap(), "fn main() { feature(); }\n");
        assert_eq!(fs::read_to_string(project.join("feature.rs")).unwrap(), "fn feature() {}\n");
        assert_eq!(Session::open(&path).unwrap().messages().len(), 2);
    }

    #[test]
    fn a_message_with_no_snapshot_rewinds_the_conversation_alone() {
        let folder = tempfile::tempdir().unwrap();
        let store = Checkpoints::new(folder.path().join("store"), folder.path().join("data"));
        let path = folder.path().join("chat.jsonl");
        let mut session = Session::create(&path, folder.path()).unwrap();
        session.push(Message::user("hello"), None).unwrap();
        drop(session);
        assert_eq!(preview(&path, 0, &store, folder.path()).unwrap(), Preview::default());
        assert_eq!(rewind(&path, 0, None).unwrap(), Files::default());
        assert!(Session::open(&path).unwrap().messages().is_empty());
        assert_eq!(undo(&path, &store, folder.path()).unwrap(), Files::default());
        assert_eq!(Session::open(&path).unwrap().messages().len(), 1);
        assert!(preview(&path, 3, &store, folder.path()).is_err());
    }
}
