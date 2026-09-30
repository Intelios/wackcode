//! Project memory (Settings › Memory): the per-project notes the agent keeps for itself.
//!
//! One project (a Git repository — every worktree of it shares one directory — or a plain
//! folder) maps to `<app data>/memory/<name>-<key>/`, where the key hashes the repository's
//! common root. Notes are `<type>_<slug>.md` files with the same frontmatter the worker's
//! `builtin/memory/store.ts` reads and writes:
//!
//! ```text
//! ---
//! type: feedback
//! title: Run worker tests
//! description: Protocol edits need pnpm test:worker
//! modified: 2026-09-28T10:12:00+00:00
//! ---
//! Body markdown…
//! ```
//!
//! A note's *name* is its filename without `.md` — the id the `memory_recall` and
//! `memory_forget` tools take. The worker builds the one-line index it serves in the system
//! prompt from these files; Settings lists, edits and deletes them here. An `origin.json`
//! beside them records which folder the directory belongs to, so Settings can label a
//! directory whose project was removed.
//!
//! Everything is machine-local and outside the workspace: no note ever enters a repository,
//! a checkpoint, or the network. Writes are guarded like the commands folder (canonical
//! `inside` check, so a symlink cannot lead out) and deletes go to the Trash.

use crate::models::{
    MemoryConfig, MemoryDocument, MemoryEntry, MemoryProject, MemoriesChange, MemoriesOverview,
    SaveMemoryInput, TaskRecord,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager};

const KEY_HEX: usize = 12;
const MAX_NAME_CHARS: usize = 80;
const MAX_TITLE_CHARS: usize = 200;
const MAX_DESCRIPTION_CHARS: usize = 300;
const MAX_BODY_CHARS: usize = 200_000;
/// The worker caps its index there; far beyond that something is wrong with the folder.
const MAX_ENTRIES_PER_PROJECT: usize = 500;
pub const MEMORY_TYPES: &[&str] = &["user", "feedback", "project", "reference"];

pub fn dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|_| "The app data folder could not be found.".to_string())?
        .join("memory"))
}

/// FNV-1a over the path, kept to 48 bits: short enough to read in a directory name, long
/// enough that two projects never share it.
fn short_hash(value: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:0width$x}", hash & 0xffff_ffff_ffff, width = KEY_HEX)
}

fn slug(text: &str) -> String {
    let mut slug = String::new();
    let mut dash = false;
    for character in text.chars() {
        let lowercase = character.to_ascii_lowercase();
        if lowercase.is_ascii_alphanumeric() {
            slug.push(lowercase);
            dash = false;
        } else if !dash && !slug.is_empty() {
            slug.push('-');
            dash = true;
        }
        if slug.chars().count() >= 24 {
            break;
        }
    }
    let trimmed = slug.trim_matches('-').to_string();
    if trimmed.is_empty() { "project".into() } else { trimmed }
}

/// The memory key of a workspace: its repository's common root when it has one (so every
/// worktree and subfolder of a repository shares memory), else the folder itself.
pub fn workspace_key(workspace: &Path) -> String {
    let anchor = crate::git::common_root(workspace)
        .or_else(|| workspace.canonicalize().ok())
        .unwrap_or_else(|| workspace.to_path_buf());
    short_hash(&anchor.to_string_lossy())
}

/// The directory name for a key: readable first, unique second.
fn dir_name(key: &str, label: &str) -> String {
    format!("{}-{}", slug(label), key)
}

/// The key a memory directory was created under, from its `<name>-<key>` folder name.
fn key_of_dir(name: &str) -> Option<String> {
    let (_, key) = name.rsplit_once('-')?;
    let valid = key.len() == KEY_HEX && key.bytes().all(|byte| byte.is_ascii_hexdigit());
    valid.then(|| key.to_lowercase())
}

#[derive(Debug, Deserialize)]
struct Origin {
    root: String,
}

/// The name a workspace's memory directory is filed under: its repository's name when it has
/// one (worktrees and subfolders share it), else the folder's own name.
fn root_label(workspace: &Path) -> String {
    let anchor = crate::git::common_root(workspace)
        .or_else(|| workspace.canonicalize().ok())
        .unwrap_or_else(|| workspace.to_path_buf());
    anchor
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".into())
}

/// This chat's project directory, created (with its `origin.json`) on first use. Called from
/// the worker's init path, so a chat's first run is what brings the directory into being.
pub fn root_for(app: &AppHandle, workspace: &Path) -> Result<PathBuf, String> {
    let key = workspace_key(workspace);
    let root = dir(app)?.join(dir_name(&key, &root_label(workspace)));
    let origin = root.join("origin.json");
    if !origin.exists() {
        fs::create_dir_all(&root).map_err(|error| format!("Could not create the memory folder: {error}"))?;
        let payload = json!({ "root": workspace.canonicalize().unwrap_or_else(|_| workspace.to_path_buf()) });
        let bytes = serde_json::to_vec(&payload).map_err(|error| error.to_string())?;
        // Best-effort: a missing origin only costs Settings its label, not the memory.
        let _ = fs::write(&origin, bytes);
    }
    Ok(root)
}

/// The `memory` value for `init` and `set_memory`: the chat's project directory plus the
/// effective on/off after the global and per-project switches. Deliberately never part of the
/// worker fingerprint — a toggle must not respawn a chat.
pub fn payload(app: &AppHandle, task: &TaskRecord) -> Result<Value, String> {
    let workspace = Path::new(&task.workspace_path);
    let root = root_for(app, workspace)?;
    let key = workspace_key(workspace);
    let enabled = {
        let state = app.state::<crate::storage::MetadataState>();
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        data.memory.enabled && !data.memory.disabled_projects.contains(&key)
    };
    Ok(json!({ "root": root, "enabled": enabled }))
}

// ---------------------------------------------------------------------------------------------
// Validation

fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the memory a name.".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("A memory name can be at most {MAX_NAME_CHARS} characters."));
    }
    let valid = |character: char| character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-' || character == '_';
    if !name.chars().all(valid) || name.starts_with(['-', '_']) {
        return Err("Use only lowercase letters, numbers, hyphens and underscores, like feedback_run-tests.".into());
    }
    Ok(name.to_string())
}

fn validate_type(kind: &str) -> Result<String, String> {
    if MEMORY_TYPES.contains(&kind) {
        Ok(kind.to_string())
    } else {
        Err(format!("The memory type must be one of {}.", MEMORY_TYPES.join(", ")))
    }
}

fn validate_title(title: &str) -> Result<String, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("Give the memory a title.".into());
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(format!("A memory title can be at most {MAX_TITLE_CHARS} characters."));
    }
    Ok(title.to_string())
}

fn validate_description(description: &str) -> Result<String, String> {
    let description = description.trim();
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(format!("A memory description can be at most {MAX_DESCRIPTION_CHARS} characters."));
    }
    Ok(description.to_string())
}

fn validate_body(body: &str) -> Result<String, String> {
    let body = body.trim();
    if body.is_empty() {
        return Err("The memory needs something to remember.".into());
    }
    if body.chars().count() > MAX_BODY_CHARS {
        return Err(format!("A memory can be at most {MAX_BODY_CHARS} characters."));
    }
    Ok(body.to_string())
}

/// A path the renderer sends back: absolute, a `.md` file, and strictly inside the memory root.
fn memory_file(path: &str, root: &Path) -> Result<PathBuf, String> {
    let path = Path::new(path);
    if path.as_os_str().len() > 4_096
        || !path.is_absolute()
        || path.components().any(|component| component == std::path::Component::ParentDir)
        || path.extension().and_then(|extension| extension.to_str()) != Some("md")
    {
        return Err("That is not a memory file.".into());
    }
    crate::skills::inside(path, root)
        .filter(|file| file.is_file())
        .ok_or_else(|| "Only notes in your memory folders can be changed here.".to_string())
}

/// A project directory the renderer names for a new note: an existing directory directly
/// under the memory root, spelled the way a scan reported it.
pub(crate) fn project_dir(dir: &str, root: &Path) -> Result<PathBuf, String> {
    let requested = Path::new(dir);
    if !requested.is_absolute()
        || requested.components().any(|component| component == std::path::Component::ParentDir)
    {
        return Err("That is not a memory folder.".into());
    }
    let canonical = requested.canonicalize().map_err(|_| "That memory folder is gone.".to_string())?;
    let parent = canonical.parent();
    let root = root.canonicalize().map_err(|_| "The memory folder could not be found.".to_string())?;
    if parent != Some(root.as_path()) || !canonical.is_dir() {
        return Err("That is not one of your memory folders.".to_string());
    }
    Ok(canonical)
}

// ---------------------------------------------------------------------------------------------
// Note text

/// One note file: the frontmatter the worker's store reads, then the body.
pub fn compose(kind: &str, title: &str, description: &str, modified: &str, body: &str) -> String {
    let mut out = String::from("---\n");
    out.push_str(&format!("type: {}\n", kind));
    out.push_str(&format!("title: {}\n", crate::skills::yaml_scalar(title)));
    out.push_str(&format!("description: {}\n", crate::skills::yaml_scalar(description)));
    out.push_str(&format!("modified: {}\n", modified));
    out.push_str("---\n\n");
    out.push_str(body.trim());
    out.push('\n');
    out
}

/// A frontmatter value as the editor sees it: a plain `key: value` line, quotes stripped.
fn field(yaml: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    for line in yaml.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix(&prefix) {
            let value = rest.trim();
            let unquoted = if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
                value[1..value.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\")
            } else {
                value.to_string()
            };
            return Some(unquoted);
        }
    }
    None
}

/// Every valid note in one project directory, alphabetical by name. Dot-files (the worker's
/// staging files) and the trash folder never appear.
fn scan_project(directory: &Path) -> Vec<MemoryEntry> {
    let Ok(files) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut names: Vec<String> = files
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".md") && !name.starts_with('.'))
        .collect();
    names.sort();
    let mut entries = Vec::new();
    for name in names {
        let text = fs::read_to_string(directory.join(&name)).unwrap_or_default();
        let (frontmatter, _) = crate::skills::split(&text);
        let yaml = frontmatter.unwrap_or_default();
        let Some(title) = field(&yaml, "title").filter(|title| !title.trim().is_empty()) else {
            continue;
        };
        let kind = field(&yaml, "type").unwrap_or_else(|| "project".into());
        if !MEMORY_TYPES.contains(&kind.as_str()) {
            continue;
        }
        entries.push(MemoryEntry {
            name: name.strip_suffix(".md").unwrap_or(&name).to_string(),
            file_path: directory.join(&name).display().to_string(),
            kind,
            title,
            description: field(&yaml, "description").unwrap_or_default(),
            modified: field(&yaml, "modified"),
        });
        if entries.len() >= MAX_ENTRIES_PER_PROJECT {
            break;
        }
    }
    entries
}

/// Settings' list: every project directory under the memory root, labelled by its project
/// when one is known (its key matches), else by the folder `origin.json` names.
pub fn overview(app: &AppHandle) -> Result<MemoriesOverview, String> {
    let root = dir(app)?;
    let (config, projects) = {
        let state = app.state::<crate::storage::MetadataState>();
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        (data.memory.clone(), data.projects.clone())
    };
    let known_names: Vec<(String, String)> = projects
        .iter()
        .map(|project| (workspace_key(Path::new(&project.path)), project.name.clone()))
        .collect();
    let name_of_key = |key: &str| known_names.iter().find(|(candidate, _)| candidate == key).map(|(_, name)| name.clone());

    let mut listed: Vec<MemoryProject> = Vec::new();
    if let Ok(directories) = fs::read_dir(&root) {
        for entry in directories.filter_map(|entry| entry.ok()) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Some(key) = key_of_dir(&name) else { continue };
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let origin = fs::read_to_string(path.join("origin.json"))
                .ok()
                .and_then(|text| serde_json::from_str::<Origin>(&text).ok())
                .map(|origin| origin.root)
                .unwrap_or_else(|| name.clone());
            let fallback_label = Path::new(&origin)
                .file_name()
                .map(|label| label.to_string_lossy().into_owned())
                .unwrap_or_else(|| origin.clone());
            listed.push(MemoryProject {
                key: key.clone(),
                name: name_of_key(&key).unwrap_or(fallback_label),
                path: origin,
                dir: path.display().to_string(),
                entries: scan_project(&path),
                enabled: config.enabled && !config.disabled_projects.contains(&key),
            });
        }
    }
    listed.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(MemoriesOverview { enabled: config.enabled, projects: listed })
}

/// Switch one project's memory on or off. The list stores the key exactly as the scan spelled it.
pub fn set_project_disabled(config: &mut MemoryConfig, key: &str, enabled: bool) -> Result<(), String> {
    if key_of_dir(&format!("project-{key}")).as_deref() != Some(key) {
        return Err("That memory key is not recognised.".into());
    }
    config.disabled_projects.retain(|entry| entry != key);
    if !enabled && !config.disabled_projects.iter().any(|entry| entry == key) {
        config.disabled_projects.push(key.to_string());
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Files

/// Create a note at `<dir>/<name>.md`.
pub fn create(dir: &Path, name: &str, kind: &str, title: &str, description: &str, body: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|error| format!("Could not create the memory folder: {error}"))?;
    let file = dir.join(format!("{name}.md"));
    if fs::symlink_metadata(&file).is_ok() {
        return Err(format!("A memory named {name} already exists."));
    }
    crate::skills::write_atomic(&file, &compose(kind, title, description, &Utc::now().to_rfc3339(), body))?;
    Ok(file)
}

/// Rewrite a note; renaming the note renames the file. Returns the path afterwards, spelled
/// like `path` was.
pub fn update(root: &Path, path: &str, name: &str, kind: &str, title: &str, description: &str, body: &str) -> Result<PathBuf, String> {
    let file = memory_file(path, root)?;
    let requested = PathBuf::from(path);
    let target = file.with_file_name(format!("{name}.md"));
    if target != file && fs::symlink_metadata(&target).is_ok() {
        return Err(format!("A memory named {name} already exists."));
    }
    crate::skills::write_atomic(&file, &compose(kind, title, description, &Utc::now().to_rfc3339(), body))?;
    if target != file {
        fs::rename(&file, &target).map_err(|error| format!("The memory was saved, but its file could not be renamed: {error}"))?;
    }
    Ok(if target != file { requested.with_file_name(format!("{name}.md")) } else { requested })
}

/// Move a note file to the Trash.
pub fn delete(root: &Path, path: &str) -> Result<(), String> {
    crate::skills::move_to_trash(&memory_file(path, root)?, "memory")
}

/// Move one project's whole memory folder to the Trash: the directory Settings' scan reported,
/// re-checked to be a real folder directly under the memory root. Returns its key so the
/// config's per-project switch can leave `wackcode.json` with it — otherwise a removed
/// project's key would linger there forever.
pub fn remove_project(dir: &str, root: &Path) -> Result<String, String> {
    let folder = project_dir(dir, root)?;
    let key = folder
        .file_name()
        .and_then(|name| key_of_dir(&name.to_string_lossy()))
        .ok_or_else(|| "That memory folder is not recognised.".to_string())?;
    crate::skills::move_to_trash(&folder, "memory folder")?;
    Ok(key)
}

/// A note file `open -R` may point at: the same guard as an edit, minus the is-file check's
/// strictness — a file that just moved to the Trash is still findable, a path outside the
/// memory root never is.
pub fn findable_file(root: &Path, path: &str) -> Result<PathBuf, String> {
    let requested = Path::new(path);
    if requested.as_os_str().len() > 4_096
        || !requested.is_absolute()
        || requested.components().any(|component| component == std::path::Component::ParentDir)
        || requested.extension().and_then(|extension| extension.to_str()) != Some("md")
    {
        return Err("That is not a memory file.".into());
    }
    crate::skills::inside(requested, root).ok_or_else(|| "That memory is not in your memory folders.".to_string())
}

/// The note's body, for the editor.
pub fn read_document(root: &Path, path: &str) -> Result<MemoryDocument, String> {
    let file = memory_file(path, root)?;
    let text = fs::read_to_string(&file).map_err(|error| format!("Could not read the memory: {error}"))?;
    Ok(MemoryDocument { body: crate::skills::split(&text).1 })
}

/// Validate a save the renderer sent. Returns the normalized pieces.
pub fn validated(input: &SaveMemoryInput) -> Result<(String, String, String, String, String), String> {
    Ok((
        validate_name(&input.name)?,
        validate_type(&input.memory_type)?,
        validate_title(&input.title)?,
        validate_description(&input.description)?,
        validate_body(&input.body)?,
    ))
}

/// The fresh change Settings reports after a save, delete or switch: the new scan plus the
/// config as saved.
pub fn change(app: &AppHandle, config: MemoryConfig) -> Result<MemoriesChange, String> {
    Ok(MemoriesChange { overview: overview(app)?, config })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("memory");
        (temp, path)
    }

    #[test]
    fn keys_are_short_stable_and_path_based() {
        assert_eq!(short_hash("/repos/wackcode"), short_hash("/repos/wackcode"));
        assert_eq!(short_hash("/repos/a").len(), KEY_HEX);
        assert_ne!(short_hash("/repos/a"), short_hash("/repos/b"));
        // Directory names round-trip their key, whatever the label slugs to.
        let name = dir_name("abc123456789", "My Repo!");
        assert!(name.starts_with("my-repo-"));
        assert_eq!(key_of_dir(&name).as_deref(), Some("abc123456789"));
        assert!(key_of_dir("no-key-here").is_none());
        assert!(key_of_dir("x-123z").is_none());
    }

    #[test]
    fn slug_stays_readable_and_bounded() {
        assert_eq!(slug("WackCode"), "wackcode");
        assert_eq!(slug("A  very /// odd name"), "a-very-odd-name");
        assert_eq!(slug("???"), "project");
        assert!(slug(&"x".repeat(200)).chars().count() <= 25);
    }

    #[test]
    fn composes_and_scans_notes() {
        let (_temp, root) = root();
        let file = create(&root, "feedback_run-tests", "feedback", "Run worker tests", "Protocol edits need pnpm test:worker", "Body.")
            .unwrap();
        assert!(file.ends_with("feedback_run-tests.md"));
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.starts_with("---\ntype: feedback\n"));
        assert!(text.contains("title: Run worker tests\n"));
        assert!(text.contains("description: Protocol edits need pnpm test"));
        assert!(text.contains("modified: "));
        assert!(text.ends_with("Body.\n"));

        let entries = scan_project(&root);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "feedback_run-tests");
        assert_eq!(entries[0].kind, "feedback");
        assert_eq!(entries[0].title, "Run worker tests");
        assert!(entries[0].modified.is_some());

        // Quoted titles round-trip through the field parser, like the worker's store.
        let quoted = compose("user", "Prefers: dark UI", "", &Utc::now().to_rfc3339(), "Body");
        let (frontmatter, _) = crate::skills::split(&quoted);
        assert_eq!(field(&frontmatter.unwrap(), "title").as_deref(), Some("Prefers: dark UI"));

        // A file with no title is invisible, like in the worker's index.
        fs::write(root.join("plain.md"), "just a body").unwrap();
        fs::create_dir_all(root.join(".trash")).unwrap();
        fs::write(root.join(".trash/old.md"), "---\ntitle: hidden\n---\nx").unwrap();
        fs::write(root.join(".staging.md.tmp"), "---\ntitle: hidden\n---\nx").unwrap();
        assert_eq!(scan_project(&root).len(), 1);
    }

    #[test]
    fn updates_renames_and_reads() {
        let (_temp, root) = root();
        let file = create(&root, "project_ship", "project", "Ship it", "", "v1").unwrap();
        assert_eq!(read_document(&root, file.to_str().unwrap()).unwrap().body, "v1");
        let moved = update(&root, file.to_str().unwrap(), "project_launch", "project", "Launch", "", "v2").unwrap();
        assert!(moved.ends_with("project_launch.md"));
        assert!(fs::read_to_string(&moved).unwrap().contains("v2"));
        assert!(!file.exists());
        assert!(update(&root, "outside.md", "x", "project", "t", "", "b").is_err());
    }

    #[test]
    fn refuses_writes_outside_the_memory_root() {
        let (_temp, root) = root();
        assert!(memory_file("/etc/passwd.md", &root).is_err());
        assert!(memory_file("/tmp/../etc/x.md", &root).is_err());
        assert!(read_document(&root, "/etc/hosts.md").is_err());
        assert!(project_dir("/etc", &root).is_err());
        let inside = create(&root, "user_pref", "user", "Pref", "", "b").unwrap();
        assert!(memory_file(inside.to_str().unwrap(), &root).is_ok());
        assert_eq!(findable_file(&root, inside.to_str().unwrap()).unwrap(), inside.canonicalize().unwrap());
        assert!(findable_file(&root, "/etc/hosts.md").is_err());
    }

    #[test]
    fn a_repository_and_its_worktrees_share_one_key() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str], cwd: &Path| {
            std::process::Command::new("git").arg("-C").arg(cwd).args(args)
                .output()
                .expect("git runs")
                .status
                .success()
        };
        assert!(git(&["init", "--initial-branch=main"], &repo));
        fs::write(repo.join("README.md"), "x").unwrap();
        assert!(git(&["add", "."], &repo));
        assert!(git(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-m", "init"], &repo));
        // A subfolder of the repository resolves to the repository, like a chat opened there.
        let subfolder = repo.join("packages/app");
        fs::create_dir_all(&subfolder).unwrap();
        assert_eq!(workspace_key(&subfolder), workspace_key(&repo));
        // So does a worktree: its own `.git` is a file pointing at the shared one.
        let worktree = temp.path().join("worktree");
        assert!(git(&["worktree", "add", worktree.to_str().unwrap(), "-b", "wt"], &repo));
        assert_eq!(workspace_key(&worktree), workspace_key(&repo));
        // And the label comes from the shared root, so all of them map to one directory name.
        let key = workspace_key(&repo);
        assert_eq!(root_label(&repo), "repo");
        assert_eq!(dir_name(&key, &root_label(&repo)), dir_name(&key, &root_label(&worktree)));
        // A plain folder outside any repository keys to itself and stays distinct.
        let standalone = temp.path().join("plain");
        fs::create_dir_all(&standalone).unwrap();
        assert_ne!(workspace_key(&standalone), workspace_key(&repo));
    }

    #[test]
    fn per_project_switches_round_trip() {
        let mut config = MemoryConfig::default();
        set_project_disabled(&mut config, "abc123456789", true).unwrap();
        assert!(config.disabled_projects.is_empty());
        set_project_disabled(&mut config, "abc123456789", false).unwrap();
        assert_eq!(config.disabled_projects, vec!["abc123456789".to_string()]);
        // Disabling twice keeps one entry; enabling removes it.
        set_project_disabled(&mut config, "abc123456789", false).unwrap();
        assert_eq!(config.disabled_projects.len(), 1);
        set_project_disabled(&mut config, "abc123456789", true).unwrap();
        assert!(config.disabled_projects.is_empty());
        assert!(set_project_disabled(&mut config, "not-a-key", true).is_err());
    }

    #[test]
    fn removes_a_project_folder_and_returns_its_key() {
        let (temp, root) = root();
        fs::create_dir_all(&root).unwrap();
        // The folder a scan of a removed project reports: `<label>-<key>` under the memory root.
        let folder = root.join(dir_name("abc123456789", "Old Project"));
        create(&folder, "feedback_note", "feedback", "T", "", "b").unwrap();
        assert_eq!(remove_project(folder.to_str().unwrap(), &root).unwrap(), "abc123456789");
        assert!(!folder.exists());
        // Only a directory the scan itself could list goes: nothing outside the memory root…
        assert!(remove_project("/etc", &root).is_err());
        // …and nothing whose name does not carry a key.
        let loose = root.join("not-a-key");
        fs::create_dir_all(&loose).unwrap();
        assert!(remove_project(loose.to_str().unwrap(), &root).is_err());
        assert!(loose.exists());
        let _ = temp;
    }
}
