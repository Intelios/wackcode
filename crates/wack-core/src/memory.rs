//! Where each project's memory lives, whether it is switched on, and what Settings › Memory lists.
//! The note format and every file operation are `wack_agent::memory::store`'s, re-exported here.
//!
//! - One project maps to `<data>/memory/<label>-<key>/`. The key hashes the repository's common
//!   root (every worktree and subfolder of a repository shares it) or, outside Git, the folder.
//!   The label is only for people reading the folder; the key is what is matched.
//! - An `origin.json` beside the notes records which folder it belongs to, so Settings can name a
//!   folder whose project was removed.
//! - Notes are machine-local, under the app's data and never in the workspace: no note enters a
//!   repository or the network except as the index in a run's system prompt.
//! - Version 2's notes are never read: version 3 starts with an empty memory.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

pub use wack_agent::memory::{
    MEMORY_FORGET, MEMORY_RECALL, MEMORY_SAVE, MEMORY_TOOLS,
    store::{Draft, Kind, MAX_DESCRIPTION_CHARS, MAX_NAME_CHARS, MAX_TITLE_CHARS, Note, draft_issue, note_name, now_iso, read, save_draft, slugify},
};

use crate::store::Store;

const KEY_HEX: usize = 12;
/// The index caps itself far below this; past it something is wrong with the folder.
const MAX_NOTES_PER_PROJECT: usize = 500;

/// Settings › Memory's switches. Memory is on unless the user turns it off.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryConfig {
    #[serde(default = "on")]
    pub enabled: bool,
    /// Keys of projects whose memory is off.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_projects: Vec<String>,
}

fn on() -> bool {
    true
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self { enabled: true, disabled_projects: Vec::new() }
    }
}

/// One project's memory folder as Settings shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct MemoryProject {
    pub key: String,
    /// The project's name in WackCode, else the folder's name.
    pub name: String,
    /// The folder the memory belongs to.
    pub path: String,
    /// The memory folder itself.
    pub dir: PathBuf,
    /// By name.
    pub notes: Vec<Note>,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MemoryOverview {
    pub enabled: bool,
    /// By name, ignoring case.
    pub projects: Vec<MemoryProject>,
}

impl MemoryOverview {
    pub fn notes(&self) -> usize {
        self.projects.iter().map(|project| project.notes.len()).sum()
    }
}

/// FNV-1a over the path, kept to 48 bits: short enough to read in a folder name, long enough that
/// two projects never share it.
fn short_hash(value: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:0width$x}", hash & 0xffff_ffff_ffff, width = KEY_HEX)
}

/// The label half of a folder name: ASCII, at most 24 characters, `project` when nothing is left.
fn label_slug(text: &str) -> String {
    let mut slug = String::new();
    let mut dash = false;
    for character in text.chars() {
        let lower = character.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            slug.push(lower);
            dash = false;
        } else if !dash && !slug.is_empty() {
            slug.push('-');
            dash = true;
        }
        if slug.len() >= 24 {
            break;
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() { "project".into() } else { slug.to_string() }
}

/// What a folder's memory is keyed on: its repository's common root, else the folder itself.
fn anchor(folder: &Path) -> PathBuf {
    crate::git::common_root(folder)
        .or_else(|| folder.canonicalize().ok())
        .unwrap_or_else(|| folder.to_path_buf())
}

/// The memory key of a folder.
pub fn key(folder: &Path) -> String {
    short_hash(&anchor(folder).to_string_lossy())
}

/// The key a memory folder was made under, from its `<label>-<key>` name.
fn key_of_dir(name: &str) -> Option<String> {
    let (_, key) = name.rsplit_once('-')?;
    (key.len() == KEY_HEX && key.bytes().all(|byte| byte.is_ascii_hexdigit())).then(|| key.to_lowercase())
}

fn dir_name(folder: &Path) -> String {
    let anchor = anchor(folder);
    let label = anchor.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    format!("{}-{}", label_slug(&label), short_hash(&anchor.to_string_lossy()))
}

#[derive(Deserialize)]
struct Origin {
    root: String,
}

/// A note folder's notes, by name, capped.
fn notes_in(dir: &Path) -> Vec<Note> {
    let mut notes = wack_agent::memory::store::scan(dir);
    notes.truncate(MAX_NOTES_PER_PROJECT);
    notes
}

impl Store {
    pub fn memory_folder(&self) -> PathBuf {
        self.folder().join("memory")
    }

    /// `folder`'s memory folder, made (with its `origin.json`) on first use.
    pub fn memory_project_folder(&self, folder: &Path) -> Result<PathBuf, String> {
        let root = self.memory_folder().join(dir_name(folder));
        let origin = root.join("origin.json");
        if !origin.exists() {
            fs::create_dir_all(&root).map_err(|error| format!("Could not create the memory folder: {error}"))?;
            let path = folder.canonicalize().unwrap_or_else(|_| folder.to_path_buf());
            let json = serde_json::json!({ "root": path.to_string_lossy() }).to_string();
            // A missing origin only costs Settings its label, never the memory.
            let _ = fs::write(&origin, json);
        }
        Ok(root)
    }

    /// The memory folder a run in `folder` uses, or `None` when memory is off for it (or its
    /// folder cannot be made, which a run treats the same way).
    pub fn memory_for(&self, folder: &Path) -> Option<PathBuf> {
        let config = self.memory_config();
        if !config.enabled || config.disabled_projects.contains(&key(folder)) {
            return None;
        }
        self.memory_project_folder(folder).ok()
    }

    pub fn memory_enabled(&self) -> bool {
        self.memory_config().enabled
    }

    /// The master switch. It applies from each chat's next message.
    pub fn set_memory_enabled(&mut self, enabled: bool) -> Result<(), String> {
        if self.memory_config().enabled == enabled {
            return Ok(());
        }
        self.memory_config_mut().enabled = enabled;
        self.save_state()
    }

    /// One project's switch, by the key Settings listed it under.
    pub fn set_project_memory_enabled(&mut self, key: &str, enabled: bool) -> Result<(), String> {
        if key_of_dir(&format!("project-{key}")).as_deref() != Some(key) {
            return Err("That memory key is not recognised.".into());
        }
        let config = self.memory_config_mut();
        let before = config.disabled_projects.clone();
        config.disabled_projects.retain(|entry| entry != key);
        if !enabled {
            config.disabled_projects.push(key.to_string());
        }
        if config.disabled_projects == before {
            return Ok(());
        }
        self.save_state()
    }

    /// Forgets a project's switch once its folder is gone, so the key does not linger.
    pub fn forget_memory_project(&mut self, key: &str) -> Result<(), String> {
        let config = self.memory_config_mut();
        let before = config.disabled_projects.len();
        config.disabled_projects.retain(|entry| entry != key);
        if config.disabled_projects.len() == before {
            return Ok(());
        }
        self.save_state()
    }

    /// Settings' list: every project memory folder, named by its project when one is known (its
    /// key matches), else by the folder `origin.json` names. Reads the disk: call it off the UI
    /// thread when it matters.
    pub fn memory_overview(&self) -> MemoryOverview {
        let config = self.memory_config().clone();
        let known: Vec<(String, String)> = self.projects().iter().map(|project| (key(&project.path), project.name.clone())).collect();
        let mut projects = Vec::new();
        if let Ok(entries) = fs::read_dir(self.memory_folder()) {
            for entry in entries.filter_map(|entry| entry.ok()) {
                let name = entry.file_name().to_string_lossy().into_owned();
                let dir = entry.path();
                let Some(key) = key_of_dir(&name).filter(|_| !name.starts_with('.') && dir.is_dir()) else { continue };
                let origin = fs::read_to_string(dir.join("origin.json"))
                    .ok()
                    .and_then(|text| serde_json::from_str::<Origin>(&text).ok())
                    .map(|origin| origin.root)
                    .unwrap_or_else(|| name.clone());
                let fallback = Path::new(&origin).file_name().map(|label| label.to_string_lossy().into_owned()).unwrap_or_else(|| origin.clone());
                projects.push(MemoryProject {
                    name: known.iter().find(|(candidate, _)| *candidate == key).map(|(_, name)| name.clone()).unwrap_or(fallback),
                    enabled: config.enabled && !config.disabled_projects.contains(&key),
                    key,
                    path: origin,
                    notes: notes_in(&dir),
                    dir,
                });
            }
        }
        projects.sort_by_key(|project| project.name.to_lowercase());
        MemoryOverview { enabled: config.enabled, projects }
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    fn git(folder: &Path, args: &[&str]) {
        let status = Command::new("git").arg("-C").arg(folder).args(args).output().unwrap().status;
        assert!(status.success(), "git {args:?} failed");
    }

    #[test]
    fn keys_are_short_stable_and_path_based() {
        assert_eq!(short_hash("/a"), short_hash("/a"));
        assert_ne!(short_hash("/a"), short_hash("/b"));
        assert_eq!(short_hash("/a").len(), KEY_HEX);
        assert_eq!(key_of_dir(&format!("wackcode-{}", short_hash("/a"))), Some(short_hash("/a")));
        assert_eq!(key_of_dir("wackcode-xyz"), None);
        assert_eq!(label_slug("My Project!"), "my-project");
        assert_eq!(label_slug("———"), "project");
        assert!(label_slug(&"a".repeat(40)).len() <= 24);
    }

    #[test]
    fn a_repository_its_subfolders_and_worktrees_share_one_memory() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "start"]);
        assert_eq!(key(&repo.join("src")), key(&repo));
        let worktree = temp.path().join("worktree");
        git(&repo, &["worktree", "add", "-q", worktree.to_str().unwrap(), "-b", "wt"]);
        assert_eq!(key(&worktree), key(&repo));
        assert_eq!(dir_name(&worktree), dir_name(&repo));
        assert!(dir_name(&repo).starts_with("repo-"));

        let plain = temp.path().join("plain");
        fs::create_dir_all(&plain).unwrap();
        assert_ne!(key(&plain), key(&repo));
    }

    #[test]
    fn memory_is_on_by_default_and_switches_per_project() {
        let data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let config: MemoryConfig = serde_json::from_str("{}").unwrap();
        assert!(config.enabled, "a missing switch means on");

        let mut store = Store::open(data.path()).unwrap();
        assert!(store.memory_enabled());
        let root = store.memory_for(project.path()).unwrap();
        assert!(root.join("origin.json").exists());
        assert!(root.starts_with(store.memory_folder()));

        let key = key(project.path());
        store.set_project_memory_enabled(&key, false).unwrap();
        assert_eq!(store.memory_for(project.path()), None);
        assert!(store.set_project_memory_enabled("../x", false).is_err());
        let overview = Store::open(data.path()).unwrap().memory_overview();
        assert_eq!(overview.projects.len(), 1);
        assert!(!overview.projects[0].enabled, "the switch was saved");

        store.set_project_memory_enabled(&key, true).unwrap();
        store.set_memory_enabled(false).unwrap();
        assert_eq!(store.memory_for(project.path()), None);
        store.set_memory_enabled(true).unwrap();
        assert!(store.memory_for(project.path()).is_some());
    }

    #[test]
    fn the_overview_lists_folders_with_their_notes() {
        let data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let store = Store::open(data.path()).unwrap();
        let root = store.memory_project_folder(project.path()).unwrap();
        let draft = Draft { name: "user_tabs".into(), kind: Some(Kind::User), title: "Tabs".into(), description: String::new(), body: "Prefers tabs".into() };
        save_draft(&root, None, &draft).unwrap();
        fs::create_dir_all(store.memory_folder().join("stray")).unwrap();

        let overview = store.memory_overview();
        assert!(overview.enabled);
        assert_eq!(overview.notes(), 1);
        let listed = &overview.projects[0];
        assert_eq!(listed.dir, root);
        assert_eq!(listed.notes[0].name, "user_tabs");
        assert_eq!(listed.path, project.path().canonicalize().unwrap().to_string_lossy());
        assert_eq!(listed.name, project.path().canonicalize().unwrap().file_name().unwrap().to_string_lossy());
    }
}
