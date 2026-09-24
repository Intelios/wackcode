//! Skills (Settings › Skills): the user's own skills, other tools' skill folders, what a worker
//! receives, and Browse's reading of pi.dev's catalogue.
//!
//! The user's skills live in `~/.agents/skills`, the folder Codex, OpenCode and the Pi CLI also
//! read, so a skill made here works there too. Other tools' user-level folders load only once the
//! user switches them on, and a project's own skill folders never load: Pi's discovery stays off,
//! and a worker gets only the absolute roots `payload` names.
//!
//! WackCode writes only inside `~/.agents/skills`. Every path the renderer sends back is checked
//! against the folders listed here (canonically, so a symlink can't lead out), and deleting moves
//! a skill to the Trash rather than erasing it.

use crate::models::{
    PackageRecord, PackageSearchResult, SkillDiagnostic, SkillDocument, SkillEntry, SkillFolderKind, SkillFolderView,
    SkillPackageView, SkillsConfig, SkillsOverview,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tauri::{AppHandle, Manager};
use tokio::{io::AsyncWriteExt, process::Command};
use uuid::Uuid;

pub const LIBRARY_ID: &str = "library";
const CUSTOM_PREFIX: &str = "custom:";

struct KnownFolder {
    id: &'static str,
    label: &'static str,
    relative: &'static str,
}

/// In the order that wins a name clash: the user's own skills first.
const KNOWN_FOLDERS: &[KnownFolder] = &[
    KnownFolder { id: LIBRARY_ID, label: "Your skills", relative: ".agents/skills" },
    KnownFolder { id: "claude", label: "Claude Code", relative: ".claude/skills" },
    KnownFolder { id: "codex", label: "Codex", relative: ".codex/skills" },
    KnownFolder { id: "pi", label: "Pi CLI", relative: ".pi/agent/skills" },
    KnownFolder { id: "opencode", label: "OpenCode", relative: ".config/opencode/skills" },
];

/// The Agent Skills limits (agentskills.io/specification).
const MAX_NAME_CHARS: usize = 64;
const MAX_DESCRIPTION_CHARS: usize = 1_024;
const MAX_BODY_CHARS: usize = 200_000;
const MAX_CUSTOM_FOLDERS: usize = 16;
const MAX_DISABLED: usize = 2_000;
const MAX_PATH_CHARS: usize = 4_096;
const MAX_SKILL_FILE_BYTES: u64 = 1_000_000;
const MAX_LISTED_FILES: usize = 200;
const MAX_COPY_BYTES: u64 = 50 * 1024 * 1024;
const MAX_COPY_FILES: usize = 2_000;
/// Rebuilt on every copy, never worth carrying into `~/.agents/skills`.
const SKIPPED_DIRECTORIES: &[&str] = &[".git", "node_modules"];
const SCAN_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_SEARCH_DESCRIPTION_CHARS: usize = 500;
pub const PIDEV_PAGE_SIZE: usize = 50;

/// One folder as Settings lists it and a worker may load it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    pub id: String,
    pub label: String,
    pub path: PathBuf,
    pub kind: SkillFolderKind,
    pub enabled: bool,
}

pub fn home_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().home_dir().map_err(|_| "Your home folder could not be found.".to_string())
}

pub fn library_dir(home: &Path) -> PathBuf {
    home.join(".agents/skills")
}

/// Every folder Settings lists, in the order that wins a name clash: the user's own skills, other
/// tools' folders in a fixed order, then the folders the user added.
pub fn folders(config: &SkillsConfig, home: &Path) -> Vec<Folder> {
    let switched_on = |id: &str| config.folders.iter().any(|record| record.id == id && record.enabled);
    let mut list: Vec<Folder> = KNOWN_FOLDERS.iter().map(|known| Folder {
        id: known.id.into(),
        label: known.label.into(),
        path: home.join(known.relative),
        kind: if known.id == LIBRARY_ID { SkillFolderKind::Library } else { SkillFolderKind::Tool },
        enabled: known.id == LIBRARY_ID || switched_on(known.id),
    }).collect();
    for record in &config.folders {
        let Some(path) = record.path.as_deref().filter(|_| record.id.starts_with(CUSTOM_PREFIX)) else { continue };
        list.push(Folder {
            id: record.id.clone(),
            label: custom_label(Path::new(path)),
            path: PathBuf::from(path),
            kind: SkillFolderKind::Custom,
            enabled: record.enabled,
        });
    }
    list
}

/// A generic last segment ("skills") says little on its own, so it keeps its parent's name.
fn custom_label(path: &Path) -> String {
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
    match path.parent().and_then(Path::file_name) {
        Some(parent) if name == "skills" => format!("{}/{name}", parent.to_string_lossy()),
        _ => name,
    }
}

/// What a worker loads, for `init.skills` and `set_skills`. Never part of the fingerprint.
pub fn payload(config: &SkillsConfig, home: &Path) -> Value {
    let roots: Vec<Value> = folders(config, home).into_iter()
        .filter(|folder| folder.enabled)
        .map(|folder| json!({ "path": folder.path, "label": folder.label }))
        .collect();
    json!({ "roots": roots, "disabled": config.disabled })
}

pub fn is_known_tool_folder(id: &str) -> bool {
    id != LIBRARY_ID && KNOWN_FOLDERS.iter().any(|known| known.id == id)
}

pub fn display_path(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

// ---------------------------------------------------------------------------------------------
// Validation

pub fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the skill a name.".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("A skill name can be at most {MAX_NAME_CHARS} characters."));
    }
    if !name.chars().all(|character| character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-') {
        return Err("Use only lowercase letters, numbers and hyphens in a skill name.".into());
    }
    if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        return Err("A skill name can't start or end with a hyphen, or have two in a row.".into());
    }
    Ok(name.to_string())
}

pub fn validate_description(description: &str) -> Result<String, String> {
    let description = description.trim();
    if description.is_empty() {
        return Err("Describe what the skill does and when to use it: the agent decides from this alone.".into());
    }
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(format!("A skill's description can be at most {MAX_DESCRIPTION_CHARS} characters."));
    }
    Ok(description.to_string())
}

pub fn validate_body(body: &str) -> Result<String, String> {
    if body.chars().count() > MAX_BODY_CHARS {
        return Err(format!("A skill's instructions can be at most {MAX_BODY_CHARS} characters."));
    }
    Ok(body.trim().to_string())
}

/// A skill path the renderer sends back: absolute, a Markdown file, and sane in size. Where it
/// may point is checked separately against the listed folders.
pub fn validate_skill_path(path: &str) -> Result<PathBuf, String> {
    let path = Path::new(path);
    if path.as_os_str().len() > MAX_PATH_CHARS
        || !path.is_absolute()
        || path.components().any(|component| component == std::path::Component::ParentDir)
        || path.extension().and_then(|extension| extension.to_str()) != Some("md")
    {
        return Err("That is not a skill file.".into());
    }
    Ok(path.to_path_buf())
}

/// Switch one skill on or off. The denylist keeps the path exactly as the scan spelled it.
pub fn set_disabled(config: &mut SkillsConfig, path: &str, enabled: bool) -> Result<(), String> {
    config.disabled.retain(|entry| entry != path);
    if !enabled {
        if config.disabled.len() >= MAX_DISABLED {
            return Err("Too many skills are switched off. Switch some back on first.".into());
        }
        config.disabled.push(path.to_string());
    }
    Ok(())
}

pub fn can_add_custom_folder(config: &SkillsConfig) -> Result<(), String> {
    if config.folders.iter().filter(|record| record.id.starts_with(CUSTOM_PREFIX)).count() >= MAX_CUSTOM_FOLDERS {
        return Err(format!("You can add at most {MAX_CUSTOM_FOLDERS} folders."));
    }
    Ok(())
}

pub fn custom_folder_id() -> String {
    format!("{CUSTOM_PREFIX}{}", Uuid::new_v4())
}

/// A picked folder that would scan half the disk, or that is already listed, is refused.
pub fn check_new_folder(path: &Path, config: &SkillsConfig, home: &Path) -> Result<(), String> {
    let canonical = path.canonicalize().map_err(|_| "That folder could not be read.".to_string())?;
    let home_canonical = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    if canonical == Path::new("/") || canonical == home_canonical {
        return Err("Pick the folder that holds your skills, not your whole home folder.".into());
    }
    let already = folders(config, home).into_iter()
        .any(|folder| folder.path.canonicalize().map(|existing| existing == canonical).unwrap_or(false));
    if already {
        return Err("That folder is already listed.".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Where a path may point

/// The canonical `path`, if it exists and lies strictly inside the canonical `root`.
pub fn inside(path: &Path, root: &Path) -> Option<PathBuf> {
    let root = root.canonicalize().ok()?;
    let path = path.canonicalize().ok()?;
    (path != root && path.starts_with(&root)).then_some(path)
}

/// A skill file WackCode may change: an existing Markdown file inside `~/.agents/skills`.
pub fn library_skill(path: &str, home: &Path) -> Result<PathBuf, String> {
    let path = validate_skill_path(path)?;
    inside(&path, &library_dir(home))
        .filter(|file| file.is_file())
        .ok_or_else(|| "Only skills in Your skills (~/.agents/skills) can be changed here.".to_string())
}

/// A skill file Settings may open: inside a listed folder or a trusted package.
pub fn readable_skill(path: &str, roots: &[PathBuf]) -> Result<PathBuf, String> {
    let path = validate_skill_path(path)?;
    roots.iter()
        .find_map(|root| inside(&path, root))
        .filter(|file| file.is_file())
        .ok_or_else(|| "That skill is no longer there.".to_string())
}

// ---------------------------------------------------------------------------------------------
// SKILL.md text

/// Pi's own reading (`utils/frontmatter.js`): the file must start with `---`, and the block ends
/// at the next line that starts with `---`. Returns the text between them, and the trimmed body.
pub fn split(text: &str) -> (Option<String>, String) {
    let normalized = text.strip_prefix('\u{feff}').unwrap_or(text).replace("\r\n", "\n").replace('\r', "\n");
    if !normalized.starts_with("---") {
        return (None, normalized.trim().to_string());
    }
    match normalized[3..].find("\n---") {
        None => (None, normalized.trim().to_string()),
        Some(offset) => {
            let end = offset + 3;
            let yaml = if end > 4 { normalized[4..end].to_string() } else { String::new() };
            (Some(yaml), normalized[end + 4..].trim().to_string())
        }
    }
}

/// The frontmatter keys the editor owns; every other key is carried over untouched.
const OWNED_KEYS: &[&str] = &["name", "description", "disable-model-invocation"];

/// A new `SKILL.md`, keeping every frontmatter key of `original` the editor doesn't own
/// (`license`, `metadata`, `allowed-tools`, …) exactly as written.
pub fn compose(original: Option<&str>, name: &str, description: &str, manual: bool, body: &str) -> String {
    let kept = original.and_then(|text| split(text).0).map(|yaml| other_keys(&yaml)).unwrap_or_default();
    let mut out = String::from("---\n");
    out.push_str(&format!("name: {}\n", yaml_scalar(name)));
    out.push_str(&format!("description: {}\n", yaml_scalar(description)));
    if manual {
        out.push_str("disable-model-invocation: true\n");
    }
    for line in kept {
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str("---\n");
    let body = body.trim();
    if !body.is_empty() {
        out.push('\n');
        out.push_str(body);
        out.push('\n');
    }
    out
}

/// The frontmatter's lines minus the keys the editor owns. A top-level key starts at column 0;
/// everything up to the next one (indented lines, list items, block scalars, comments) is its.
fn other_keys(yaml: &str) -> Vec<String> {
    let mut kept = Vec::new();
    let mut skipping = false;
    for line in yaml.lines() {
        if let Some(key) = top_level_key(line) {
            skipping = OWNED_KEYS.contains(&key);
        }
        if !skipping {
            kept.push(line.to_string());
        }
    }
    while kept.last().is_some_and(|line| line.trim().is_empty()) {
        kept.pop();
    }
    kept
}

fn top_level_key(line: &str) -> Option<&str> {
    let first = line.chars().next()?;
    if first.is_whitespace() || first == '#' || first == '-' {
        return None;
    }
    let (key, rest) = if first == '"' || first == '\'' {
        let close = line[1..].find(first)? + 1;
        (&line[1..close], line[close + 1..].trim_start())
    } else {
        let colon = line.find(':')?;
        (line[..colon].trim_end(), &line[colon..])
    };
    let after = rest.strip_prefix(':')?;
    (after.is_empty() || after.starts_with([' ', '\t'])).then_some(key)
}

/// One line of text as a YAML scalar: plain when that reads back as the same string in any YAML
/// reader, otherwise double-quoted with JSON's escapes, which YAML reads the same way.
fn yaml_scalar(value: &str) -> String {
    if plain_is_safe(value) {
        value.to_string()
    } else {
        serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into())
    }
}

fn plain_is_safe(value: &str) -> bool {
    let Some(first) = value.chars().next() else { return false };
    if !first.is_ascii_alphabetic() || value.ends_with([' ', ':']) {
        return false;
    }
    if value.contains(": ") || value.contains(" #") || value.chars().any(char::is_control) {
        return false;
    }
    !matches!(value.to_ascii_lowercase().as_str(), "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "y" | "n")
}

// ---------------------------------------------------------------------------------------------
// Files

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    // A leading dot keeps the half-written file out of every skill scan.
    let staging = path.with_file_name(format!(".{name}.wackcode-{}", Uuid::new_v4()));
    fs::write(&staging, text).map_err(|error| format!("Could not save the skill: {error}"))?;
    fs::rename(&staging, path).map_err(|error| {
        let _ = fs::remove_file(&staging);
        format!("Could not save the skill: {error}")
    })
}

fn taken(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn already_exists(name: &str) -> String {
    format!("A skill named {name} already exists in Your skills.")
}

/// A new skill in its own folder, `~/.agents/skills/<name>/SKILL.md`.
pub fn create_skill(home: &Path, name: &str, description: &str, manual: bool, body: &str) -> Result<PathBuf, String> {
    let library = library_dir(home);
    fs::create_dir_all(&library).map_err(|error| format!("Could not create ~/.agents/skills: {error}"))?;
    let folder = library.join(name);
    if taken(&folder) {
        return Err(already_exists(name));
    }
    fs::create_dir(&folder).map_err(|error| format!("Could not create the skill's folder: {error}"))?;
    let file = folder.join("SKILL.md");
    if let Err(error) = write_atomic(&file, &compose(None, name, description, manual, body)) {
        let _ = fs::remove_dir_all(&folder);
        return Err(error);
    }
    Ok(file)
}

/// Rewrite a skill in the library. A skill's own folder directly inside the library follows its
/// name, so renaming the skill renames the folder too. Returns the file's path afterwards,
/// spelled like `path` was.
pub fn update_skill(home: &Path, path: &str, name: &str, description: &str, manual: bool, body: &str) -> Result<PathBuf, String> {
    let file = library_skill(path, home)?;
    let library = library_dir(home).canonicalize().map_err(|error| error.to_string())?;
    let metadata = fs::metadata(&file).map_err(|error| format!("Could not read the skill: {error}"))?;
    if metadata.len() > MAX_SKILL_FILE_BYTES {
        return Err("This skill file is too large to edit here.".into());
    }
    let original = fs::read_to_string(&file).map_err(|error| format!("Could not read the skill: {error}"))?;
    let is_skill_md = file.file_name().is_some_and(|file_name| file_name == "SKILL.md");
    let rename = file.parent()
        .filter(|folder| is_skill_md && folder.parent() == Some(library.as_path()))
        .filter(|folder| folder.file_name().is_some_and(|folder_name| folder_name != name))
        .map(|folder| (folder.to_path_buf(), library.join(name)));
    if let Some((_, target)) = &rename {
        if taken(target) {
            return Err(already_exists(name));
        }
    }
    write_atomic(&file, &compose(Some(&original), name, description, manual, body))?;
    let requested = PathBuf::from(path);
    match rename {
        Some((from, to)) => {
            fs::rename(&from, &to).map_err(|error| format!("The skill was saved, but its folder could not be renamed: {error}"))?;
            Ok(requested.parent().and_then(Path::parent).map(|parent| parent.join(name).join("SKILL.md")).unwrap_or(to.join("SKILL.md")))
        }
        None => Ok(requested),
    }
}

/// Move a skill in the library to the Trash: its folder, or just the file for a loose `.md`.
pub fn delete_skill(home: &Path, path: &str) -> Result<(), String> {
    let file = library_skill(path, home)?;
    let library = library_dir(home).canonicalize().map_err(|error| error.to_string())?;
    let is_skill_md = file.file_name().is_some_and(|name| name == "SKILL.md");
    let target = match file.parent() {
        Some(folder) if is_skill_md && folder != library => folder.to_path_buf(),
        _ => file,
    };
    move_to_trash(&target)
}

fn move_to_trash(path: &Path) -> Result<(), String> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, None)
        .map_err(|error| format!("Could not move the skill to the Trash: {}", error.localizedDescription()))
}

/// The instructions and the folder's other files, for the editor.
pub fn read_document(file: &Path) -> Result<SkillDocument, String> {
    let metadata = fs::metadata(file).map_err(|error| format!("Could not read the skill: {error}"))?;
    if metadata.len() > MAX_SKILL_FILE_BYTES {
        return Err("This skill file is too large to open here.".into());
    }
    let text = fs::read_to_string(file).map_err(|error| format!("Could not read the skill: {error}"))?;
    let (_, body) = split(&text);
    let (files, files_truncated) = match file.parent() {
        Some(folder) if file.file_name().is_some_and(|name| name == "SKILL.md") => list_files(folder),
        _ => (Vec::new(), false),
    };
    Ok(SkillDocument { body, files, files_truncated })
}

fn list_files(folder: &Path) -> (Vec<String>, bool) {
    let mut files = Vec::new();
    let mut pending = vec![folder.to_path_buf()];
    let mut truncated = false;
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || SKIPPED_DIRECTORIES.contains(&name.as_str()) {
                continue;
            }
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                pending.push(path);
                continue;
            }
            let relative = path.strip_prefix(folder).map(|relative| relative.display().to_string()).unwrap_or(name);
            if relative == "SKILL.md" {
                continue;
            }
            if files.len() >= MAX_LISTED_FILES {
                truncated = true;
                break;
            }
            files.push(relative);
        }
    }
    files.sort();
    (files, truncated)
}

/// Copy one skill into `~/.agents/skills/<name>`: its whole folder for a `SKILL.md`, or a loose
/// `.md` file as that folder's `SKILL.md`. The copy is staged under a dot-name, which no scan
/// reads, and appears in one rename.
pub fn copy_into_library(home: &Path, name: &str, file: &Path, base_dir: &Path) -> Result<PathBuf, String> {
    let name = validate_name(name).map_err(|_| format!("“{name}” isn't a valid skill name, so it wasn't copied."))?;
    let library = library_dir(home);
    fs::create_dir_all(&library).map_err(|error| format!("Could not create ~/.agents/skills: {error}"))?;
    let target = library.join(&name);
    if taken(&target) {
        return Err(already_exists(&name));
    }
    let staging = library.join(format!(".wackcode-import-{}", Uuid::new_v4()));
    let copied = if file.file_name().is_some_and(|file_name| file_name == "SKILL.md") {
        copy_tree(base_dir, &staging)
    } else {
        fs::create_dir(&staging)
            .and_then(|_| fs::copy(file, staging.join("SKILL.md")).map(|_| ()))
            .map_err(|error| format!("Could not copy {name}: {error}"))
    };
    let result = copied.and_then(|_| fs::rename(&staging, &target).map_err(|error| format!("Could not copy {name}: {error}")));
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result.map(|_| target.join("SKILL.md"))
}

/// Everything in `from` except symlinks, dot-git and `node_modules`, within the copy limits,
/// which are checked before anything is written.
fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    let mut directories = Vec::new();
    let mut files = Vec::new();
    let mut bytes = 0u64;
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let entries = fs::read_dir(from.join(&relative)).map_err(|error| format!("Could not read {}: {error}", from.join(&relative).display()))?;
        directories.push(relative.clone());
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_symlink() {
                continue;
            }
            let child = relative.join(&name);
            if kind.is_dir() {
                if !SKIPPED_DIRECTORIES.contains(&name.to_string_lossy().as_ref()) {
                    pending.push(child);
                }
            } else if kind.is_file() {
                bytes += entry.metadata().map(|metadata| metadata.len()).unwrap_or(0);
                files.push(child);
                if files.len() > MAX_COPY_FILES || bytes > MAX_COPY_BYTES {
                    return Err(format!(
                        "That skill is too large to copy (at most {MAX_COPY_FILES} files and {} MB).",
                        MAX_COPY_BYTES / 1024 / 1024
                    ));
                }
            }
        }
    }
    for directory in &directories {
        fs::create_dir_all(to.join(directory)).map_err(|error| format!("Could not copy the skill: {error}"))?;
    }
    for file in &files {
        fs::copy(from.join(file), to.join(file)).map_err(|error| format!("Could not copy {}: {error}", file.display()))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Scanning

#[derive(Debug, Deserialize)]
struct ScanLine {
    ok: bool,
    #[serde(default)]
    folders: Vec<ScannedGroup>,
    #[serde(default)]
    packages: Vec<ScannedGroup>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ScannedGroup {
    id: String,
    #[serde(default)]
    skills: Vec<ScannedSkill>,
    #[serde(default)]
    diagnostics: Vec<ScannedDiagnostic>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedSkill {
    pub name: String,
    pub description: String,
    pub file_path: String,
    pub base_dir: String,
    #[serde(default)]
    pub manual: bool,
    #[serde(default)]
    resource_name: Option<String>,
    #[serde(default)]
    shadowed_by: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ScannedDiagnostic {
    #[serde(rename = "type")]
    kind: String,
    message: String,
    #[serde(default)]
    path: Option<String>,
}

/// Run Pi's own skill loader over `folders` and the trusted packages' skills in a short-lived
/// process (`skills-scan.js`) that only reads files: no session, no key, no network.
async fn run_scan(app: &AppHandle, folders: &[Folder], packages: &[&PackageRecord], disabled: &[String]) -> Result<ScanLine, String> {
    let request = json!({
        "folders": folders.iter().map(|folder| json!({
            "id": folder.id, "path": folder.path, "label": folder.label, "enabled": folder.enabled,
        })).collect::<Vec<_>>(),
        "packages": packages.iter().map(|package| json!({
            "source": package.source,
            "label": package.display_name,
            "resources": package.skills.iter().map(|resource| json!({
                "path": resource.path, "name": resource.name, "enabled": resource.enabled,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "disabled": disabled,
    });
    // Only used by Pi to tell "user" skills from others; nothing is read from or written to it.
    let agent_dir = app.path().app_data_dir().map_err(|error| error.to_string())?.join("agent");
    let mut command = Command::new(crate::worker::node_executable_path()?);
    command
        .arg(crate::worker::skills_scan_entry_path(app)?)
        .current_dir(home_dir(app)?)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .env("PI_TELEMETRY", "0")
        .env("PI_SKIP_VERSION_CHECK", "1")
        .env("PI_OFFLINE", "1");
    crate::worker::strip_provider_env(&mut command);
    let mut child = command.spawn().map_err(|error| format!("Could not read your skills: {error}"))?;
    let mut stdin = child.stdin.take().ok_or("Could not read your skills.")?;
    let mut line = serde_json::to_vec(&json!({ "request": request, "agentDir": agent_dir })).map_err(|error| error.to_string())?;
    line.push(b'\n');
    stdin.write_all(&line).await.map_err(|error| format!("Could not read your skills: {error}"))?;
    drop(stdin);
    let output = tokio::time::timeout(SCAN_TIMEOUT, child.wait_with_output()).await
        .map_err(|_| "Reading your skill folders took too long. A folder may be very large.".to_string())?
        .map_err(|error| format!("Could not read your skills: {error}"))?;
    let result = String::from_utf8_lossy(&output.stdout)
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<ScanLine>(line).ok())
        .ok_or("Reading your skills stopped without an answer.")?;
    if !result.ok {
        return Err(format!(
            "Could not read your skills: {}",
            crate::worker::redact_and_limit(result.error.as_deref().unwrap_or("unknown error"))
        ));
    }
    Ok(result)
}

/// Settings' list: every folder, switched on or not, and every trusted package's skills.
pub async fn overview(app: &AppHandle) -> Result<SkillsOverview, String> {
    let home = home_dir(app)?;
    let (config, packages) = {
        let state = app.state::<crate::storage::MetadataState>();
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        (data.skills.clone(), data.packages.clone())
    };
    let listed = folders(&config, &home);
    let trusted: Vec<&PackageRecord> = packages.iter()
        .filter(|package| !package.trusted_at.is_empty() && !package.skills.is_empty())
        .collect();
    let scanned = run_scan(app, &listed, &trusted, &config.disabled).await?;
    Ok(build_overview(&home, &config, &listed, &trusted, scanned))
}

fn build_overview(home: &Path, config: &SkillsConfig, listed: &[Folder], packages: &[&PackageRecord], scanned: ScanLine) -> SkillsOverview {
    let library = library_dir(home);
    let diagnostics = |group: &ScannedGroup| group.diagnostics.iter().map(|diagnostic| SkillDiagnostic {
        kind: diagnostic.kind.clone(),
        message: diagnostic.message.clone(),
        path: diagnostic.path.clone(),
    }).collect::<Vec<_>>();

    let folders = listed.iter().map(|folder| {
        let group = scanned.folders.iter().find(|group| group.id == folder.id);
        SkillFolderView {
            id: folder.id.clone(),
            label: folder.label.clone(),
            path: folder.path.display().to_string(),
            display_path: display_path(&folder.path, home),
            kind: folder.kind,
            exists: folder.path.is_dir(),
            enabled: folder.enabled,
            skills: group.map(|group| group.skills.iter().map(|skill| SkillEntry {
                name: skill.name.clone(),
                description: skill.description.clone(),
                file_path: skill.file_path.clone(),
                base_dir: skill.base_dir.clone(),
                manual: skill.manual,
                enabled: !config.disabled.contains(&skill.file_path),
                editable: folder.kind == SkillFolderKind::Library && inside(Path::new(&skill.file_path), &library).is_some(),
                shadowed_by: skill.shadowed_by.clone(),
                resource_name: None,
            }).collect()).unwrap_or_default(),
            diagnostics: group.map(diagnostics).unwrap_or_default(),
        }
    }).collect();

    let packages = packages.iter().map(|package| {
        let group = scanned.packages.iter().find(|group| group.id == package.source);
        SkillPackageView {
            source: package.source.clone(),
            label: package.display_name.clone(),
            skills: group.map(|group| group.skills.iter().map(|skill| SkillEntry {
                name: skill.name.clone(),
                description: skill.description.clone(),
                file_path: skill.file_path.clone(),
                base_dir: skill.base_dir.clone(),
                manual: skill.manual,
                enabled: skill.resource_name.as_ref()
                    .and_then(|name| package.skills.iter().find(|resource| &resource.name == name))
                    .is_some_and(|resource| resource.enabled),
                editable: false,
                shadowed_by: skill.shadowed_by.clone(),
                resource_name: skill.resource_name.clone(),
            }).collect()).unwrap_or_default(),
            diagnostics: group.map(diagnostics).unwrap_or_default(),
        }
    }).collect();

    SkillsOverview { library_path: library.display().to_string(), folders, packages }
}

/// Skills found at `path` (a folder, or one `.md` file) for an import: nothing is loaded yet.
pub async fn scan_for_import(app: &AppHandle, path: &Path) -> Result<Vec<ScannedSkill>, String> {
    let folder = Folder {
        id: "import".into(),
        label: "Import".into(),
        path: path.to_path_buf(),
        kind: SkillFolderKind::Custom,
        enabled: false,
    };
    let mut scanned = run_scan(app, std::slice::from_ref(&folder), &[], &[]).await?;
    Ok(scanned.folders.pop().map(|group| group.skills).unwrap_or_default())
}

// ---------------------------------------------------------------------------------------------
// pi.dev

/// pi.dev's catalogue has no API (its `/api` routes answer 501), but every card on
/// `/packages?type=skill` carries its package as `data-package-*` attributes, which is what this
/// reads. Zero cards on a page without the catalogue's own empty-state message means the page
/// changed shape, and the caller falls back to npm.
pub fn parse_pidev(html: &str) -> Result<Vec<PackageSearchResult>, String> {
    let mut results = Vec::new();
    let mut cards = 0usize;
    let mut rest = html;
    while let Some(start) = rest.find("<article") {
        let card = &rest[start..];
        let Some(tag_end) = card.find('>') else { break };
        let end = card.find("</article>").unwrap_or(card.len()).max(tag_end);
        let tag = &card[..tag_end];
        let body = &card[tag_end..end];
        rest = &card[end..];
        if !tag.contains("data-package-card") {
            continue;
        }
        cards += 1;
        let Some(name) = attribute(tag, "data-package-name").map(decode_entities).filter(|name| valid_npm_name(name)) else { continue };
        let types = attribute(tag, "data-package-types")
            .map(|types| types.split_whitespace()
                .filter(|kind| kind.len() <= 20 && kind.chars().all(|character| character.is_ascii_lowercase()))
                .map(str::to_string)
                .collect())
            .unwrap_or_default();
        let published_at = attribute(tag, "data-package-date")
            .and_then(|millis| millis.parse::<i64>().ok())
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|date| date.to_rfc3339())
            .unwrap_or_default();
        let version = body.find("package-version=")
            .map(|index| &body[index + "package-version=".len()..])
            .map(|tail| tail.split(['&', '"', '\'']).next().unwrap_or_default())
            .filter(|version| version.len() <= 64 && version.chars().all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '+')))
            .unwrap_or_default()
            .to_string();
        results.push(PackageSearchResult {
            npm_url: format!("https://www.npmjs.com/package/{name}"),
            repository: None,
            version,
            description: element_text(body, "packages-desc").unwrap_or_default(),
            publisher: element_text(body, "packages-meta").map(|meta| meta.split(" · ").next().unwrap_or_default().to_string()).unwrap_or_default(),
            published_at,
            declares: Vec::new(),
            types,
            downloads: attribute(tag, "data-package-downloads").and_then(|downloads| downloads.parse().ok()),
            name,
        });
        if results.len() >= PIDEV_PAGE_SIZE {
            break;
        }
    }
    if results.is_empty() && !(cards == 0 && html.contains("packages-empty")) {
        return Err("pi.dev's catalogue page could not be read.".into());
    }
    Ok(results)
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!(" {name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(&tag[start..end])
}

/// The text of the first element with `class="<class>"`, its child elements' texts joined by
/// " · " (so `<span>a</span><span>b</span>` reads "a · b").
fn element_text(body: &str, class: &str) -> Option<String> {
    let marker = format!("class=\"{class}\"");
    let start = body.find(&marker)?;
    let open_end = body[start..].find('>')? + start + 1;
    let tag_name_start = body[..start].rfind('<')? + 1;
    let tag_name: String = body[tag_name_start..start].chars().take_while(|character| character.is_ascii_alphanumeric()).collect();
    let close = format!("</{tag_name}>");
    let end = body[open_end..].find(&close).map(|index| index + open_end).unwrap_or(body.len());
    let inner = &body[open_end..end];
    let mut parts: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut in_tag = false;
    for character in inner.chars() {
        match character {
            '<' => {
                in_tag = true;
                if !text.trim().is_empty() {
                    parts.push(text.trim().to_string());
                }
                text.clear();
            }
            '>' => in_tag = false,
            _ if !in_tag => text.push(character),
            _ => {}
        }
    }
    if !text.trim().is_empty() {
        parts.push(text.trim().to_string());
    }
    let joined = decode_entities(&parts.join(" · "));
    let collapsed = joined.split_whitespace().collect::<Vec<_>>().join(" ");
    (!collapsed.is_empty()).then(|| collapsed.chars().take(MAX_SEARCH_DESCRIPTION_CHARS).collect())
}

fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let Some(end) = tail.find(';').filter(|end| *end <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let entity = &tail[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|decimal| decimal.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(character) => {
                out.push(character);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// npm's rules, loosely: lowercase, at most 214 characters, optionally `@scope/name`.
pub fn valid_npm_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 214
        && !name.contains("..")
        && name.chars().all(|character| character.is_ascii_lowercase() || character.is_ascii_digit() || matches!(character, '-' | '.' | '_' | '~' | '@' | '/'))
        && (name.starts_with('@') == name.contains('/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{PackageResourceRecord, SkillFolderRecord};
    use std::os::unix::fs::symlink;

    fn config(folders: &[(&str, Option<&str>, bool)], disabled: &[&str]) -> SkillsConfig {
        SkillsConfig {
            folders: folders.iter().map(|(id, path, enabled)| SkillFolderRecord {
                id: (*id).into(), path: path.map(str::to_string), enabled: *enabled,
            }).collect(),
            disabled: disabled.iter().map(|path| (*path).into()).collect(),
        }
    }

    #[test]
    fn your_skills_always_load_first_and_other_folders_only_when_switched_on() {
        let home = Path::new("/Users/test");
        let config = config(&[("claude", None, true), ("codex", None, false), ("custom:1", Some("/work/team/skills"), true), ("custom:2", Some("/work/off"), false)], &["/x/SKILL.md"]);
        let listed = folders(&config, home);
        assert_eq!(listed.iter().map(|folder| folder.id.as_str()).collect::<Vec<_>>(), ["library", "claude", "codex", "pi", "opencode", "custom:1", "custom:2"]);
        assert_eq!(listed[5].label, "team/skills");
        let payload = payload(&config, home);
        let roots: Vec<&str> = payload["roots"].as_array().unwrap().iter().map(|root| root["path"].as_str().unwrap()).collect();
        assert_eq!(roots, ["/Users/test/.agents/skills", "/Users/test/.claude/skills", "/work/team/skills"]);
        assert_eq!(payload["roots"][1]["label"], "Claude Code");
        assert_eq!(payload["disabled"], json!(["/x/SKILL.md"]));
    }

    #[test]
    fn nothing_but_your_skills_loads_by_default() {
        let payload = payload(&SkillsConfig::default(), Path::new("/Users/test"));
        assert_eq!(payload["roots"].as_array().unwrap().len(), 1);
        assert_eq!(payload["roots"][0]["label"], "Your skills");
    }

    #[test]
    fn a_custom_record_without_the_custom_prefix_never_becomes_a_root() {
        let config = config(&[("claude", Some("/etc"), true)], &[]);
        let roots = payload(&config, Path::new("/Users/test"))["roots"].clone();
        assert!(!roots.as_array().unwrap().iter().any(|root| root["path"] == "/etc"));
    }

    #[test]
    fn names_follow_the_agent_skills_rules() {
        assert_eq!(validate_name(" pdf-tools ").unwrap(), "pdf-tools");
        assert!(validate_name("7zip").is_ok());
        for bad in ["", "PDF", "pdf tools", "-pdf", "pdf-", "pdf--tools", "../x", "a/b", &"a".repeat(65)] {
            assert!(validate_name(bad).is_err(), "{bad} was accepted");
        }
        assert!(validate_description("  ").is_err());
        assert!(validate_description(&"d".repeat(1_024)).is_ok());
        assert!(validate_description(&"d".repeat(1_025)).is_err());
        assert!(validate_body(&"b".repeat(MAX_BODY_CHARS + 1)).is_err());
    }

    #[test]
    fn split_reads_frontmatter_the_way_pi_does() {
        let (yaml, body) = split("\u{feff}---\r\nname: a\r\ndescription: b\r\n---\r\n\r\n# Body\r\n");
        assert_eq!(yaml.as_deref(), Some("name: a\ndescription: b"));
        assert_eq!(body, "# Body");
        assert_eq!(split("# No frontmatter\n"), (None, "# No frontmatter".into()));
        assert_eq!(split("---\nname: a\nnever closed"), (None, "---\nname: a\nnever closed".into()));
        assert_eq!(split("---\n---\nbody").1, "body");
    }

    #[test]
    fn compose_keeps_every_key_the_editor_does_not_own() {
        let original = "---\nname: old\ndescription: >\n  Folded old\n  description.\nlicense: Apache-2.0\nmetadata:\n  author: someone\n  tags:\n  - a\n  - b\ndisable-model-invocation: true\nallowed-tools: Bash(git:*) Read\n---\n\nOld body.\n";
        let text = compose(Some(original), "new-name", "Does things. Use when needed.", false, "\nNew body.\n\n");
        assert_eq!(text, "---\nname: new-name\ndescription: Does things. Use when needed.\nlicense: Apache-2.0\nmetadata:\n  author: someone\n  tags:\n  - a\n  - b\nallowed-tools: Bash(git:*) Read\n---\n\nNew body.\n");
        let manual = compose(Some(&text), "new-name", "Does things.", true, "");
        assert!(manual.contains("\ndisable-model-invocation: true\nlicense: Apache-2.0\n"));
        assert!(manual.ends_with("allowed-tools: Bash(git:*) Read\n---\n"));
        let (yaml, body) = split(&manual);
        assert!(yaml.unwrap().starts_with("name: new-name\n"));
        assert_eq!(body, "");
    }

    #[test]
    fn a_block_list_under_an_owned_key_goes_with_it() {
        let original = "---\nname: x\ndescription:\n- odd\n- list\nlicense: MIT\n---\n";
        assert!(!compose(Some(original), "x", "d", false, "").contains("odd"));
        assert!(compose(Some(original), "x", "d", false, "").contains("license: MIT"));
    }

    #[test]
    fn risky_scalars_are_quoted_and_ordinary_text_stays_plain() {
        assert_eq!(yaml_scalar("Extracts text from PDFs, fast."), "Extracts text from PDFs, fast.");
        assert_eq!(yaml_scalar("The user's notes (v2) [draft]"), "The user's notes (v2) [draft]");
        assert_eq!(yaml_scalar("Use when: always"), "\"Use when: always\"");
        assert_eq!(yaml_scalar("Tagged #1 and # comment"), "\"Tagged #1 and # comment\"");
        assert_eq!(yaml_scalar("yes"), "\"yes\"");
        assert_eq!(yaml_scalar("123"), "\"123\"");
        assert_eq!(yaml_scalar("- list-looking"), "\"- list-looking\"");
        assert_eq!(yaml_scalar("\"quoted\""), "\"\\\"quoted\\\"\"");
        assert_eq!(yaml_scalar("two\nlines"), "\"two\\nlines\"");
        assert_eq!(yaml_scalar("ends with colon:"), "\"ends with colon:\"");
    }

    #[test]
    fn only_skills_inside_your_skills_can_be_changed() {
        let home = tempfile::tempdir().unwrap();
        let library = library_dir(home.path());
        fs::create_dir_all(library.join("mine")).unwrap();
        fs::write(library.join("mine/SKILL.md"), "---\nname: mine\ndescription: d\n---\n").unwrap();
        let elsewhere = home.path().join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(elsewhere.join("SKILL.md"), "x").unwrap();
        symlink(&elsewhere, library.join("linked")).unwrap();

        assert!(library_skill(&library.join("mine/SKILL.md").display().to_string(), home.path()).is_ok());
        for bad in [
            library.join("mine/../mine/SKILL.md"),
            elsewhere.join("SKILL.md"),
            library.join("linked/SKILL.md"),
            library.join("../skills/../../elsewhere/SKILL.md"),
            library.join("missing/SKILL.md"),
            library.join("mine"),
        ] {
            assert!(library_skill(&bad.display().to_string(), home.path()).is_err(), "{} was allowed", bad.display());
        }
        assert!(library_skill("relative/SKILL.md", home.path()).is_err());
    }

    #[test]
    fn creating_renaming_and_copying_keep_one_folder_per_skill() {
        let home = tempfile::tempdir().unwrap();
        let library = library_dir(home.path());
        let file = create_skill(home.path(), "draft", "Drafts things.", false, "Do it.").unwrap();
        assert_eq!(file, library.join("draft/SKILL.md"));
        assert!(create_skill(home.path(), "draft", "Again.", false, "").unwrap_err().contains("already exists"));
        fs::write(library.join("draft/notes.txt"), "keep").unwrap();

        let renamed = update_skill(home.path(), &file.display().to_string(), "final", "Finishes things.", true, "Done.").unwrap();
        assert_eq!(renamed, library.join("final/SKILL.md"));
        assert!(!library.join("draft").exists());
        assert_eq!(fs::read_to_string(library.join("final/notes.txt")).unwrap(), "keep");
        let text = fs::read_to_string(&renamed).unwrap();
        assert_eq!(text, "---\nname: final\ndescription: Finishes things.\ndisable-model-invocation: true\n---\n\nDone.\n");

        create_skill(home.path(), "other", "Other.", false, "").unwrap();
        assert!(update_skill(home.path(), &renamed.display().to_string(), "other", "Clash.", false, "").unwrap_err().contains("already exists"));
        assert!(fs::read_to_string(&renamed).unwrap().contains("Finishes things."), "a refused rename must not rewrite the file");

        let source = home.path().join("from-claude/pdf");
        fs::create_dir_all(source.join("scripts")).unwrap();
        fs::create_dir_all(source.join("node_modules/dep")).unwrap();
        fs::write(source.join("SKILL.md"), "---\nname: pdf\ndescription: PDFs.\n---\n").unwrap();
        fs::write(source.join("scripts/run.sh"), "echo").unwrap();
        fs::write(source.join("node_modules/dep/index.js"), "x").unwrap();
        symlink("/etc/hosts", source.join("hosts")).unwrap();
        let copied = copy_into_library(home.path(), "pdf", &source.join("SKILL.md"), &source).unwrap();
        assert_eq!(copied, library.join("pdf/SKILL.md"));
        assert!(library.join("pdf/scripts/run.sh").is_file());
        assert!(!library.join("pdf/node_modules").exists());
        assert!(!library.join("pdf/hosts").exists());
        assert!(copy_into_library(home.path(), "pdf", &source.join("SKILL.md"), &source).unwrap_err().contains("already exists"));
        assert!(copy_into_library(home.path(), "Bad Name", &source.join("SKILL.md"), &source).is_err());

        let loose = home.path().join("loose.md");
        fs::write(&loose, "---\nname: loose\ndescription: Loose.\n---\n").unwrap();
        let copied = copy_into_library(home.path(), "loose", &loose, home.path()).unwrap();
        assert_eq!(fs::read_to_string(copied).unwrap(), fs::read_to_string(&loose).unwrap());
        assert!(fs::read_dir(&library).unwrap().flatten().all(|entry| !entry.file_name().to_string_lossy().starts_with(".wackcode")));
    }

    #[test]
    fn a_skill_over_the_copy_limits_is_refused_before_anything_is_written() {
        let home = tempfile::tempdir().unwrap();
        let source = home.path().join("big");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("SKILL.md"), "x").unwrap();
        for index in 0..=MAX_COPY_FILES {
            fs::write(source.join(format!("{index}.txt")), "").unwrap();
        }
        assert!(copy_into_library(home.path(), "big", &source.join("SKILL.md"), &source).unwrap_err().contains("too large"));
        assert!(!library_dir(home.path()).join("big").exists());
    }

    #[test]
    fn the_document_lists_the_skill_folder_without_skill_md() {
        let home = tempfile::tempdir().unwrap();
        let file = create_skill(home.path(), "doc", "Docs.", false, "Body text.").unwrap();
        let folder = file.parent().unwrap();
        fs::create_dir_all(folder.join("references")).unwrap();
        fs::write(folder.join("references/api.md"), "").unwrap();
        fs::write(folder.join(".hidden"), "").unwrap();
        let document = read_document(&file).unwrap();
        assert_eq!(document.body, "Body text.");
        assert_eq!(document.files, ["references/api.md"]);
        assert!(!document.files_truncated);
    }

    #[test]
    fn readable_skills_stay_inside_the_listed_roots() {
        let home = tempfile::tempdir().unwrap();
        let file = create_skill(home.path(), "doc", "Docs.", false, "").unwrap();
        let roots = vec![library_dir(home.path())];
        assert!(readable_skill(&file.display().to_string(), &roots).is_ok());
        let outside = home.path().join("outside.md");
        fs::write(&outside, "").unwrap();
        assert!(readable_skill(&outside.display().to_string(), &roots).is_err());
    }

    #[test]
    fn a_new_folder_must_be_specific_and_new() {
        let home = tempfile::tempdir().unwrap();
        let team = home.path().join("team");
        fs::create_dir_all(&team).unwrap();
        fs::create_dir_all(library_dir(home.path())).unwrap();
        let config = config(&[("custom:1", Some(&team.display().to_string()), true)], &[]);
        assert!(check_new_folder(home.path(), &config, home.path()).is_err());
        assert!(check_new_folder(Path::new("/"), &config, home.path()).is_err());
        assert!(check_new_folder(&team, &config, home.path()).unwrap_err().contains("already"));
        assert!(check_new_folder(&library_dir(home.path()), &config, home.path()).unwrap_err().contains("already"));
        let other = home.path().join("other");
        fs::create_dir_all(&other).unwrap();
        assert!(check_new_folder(&other, &config, home.path()).is_ok());
    }

    #[test]
    fn switched_off_skills_are_a_bounded_denylist() {
        let mut config = SkillsConfig::default();
        set_disabled(&mut config, "/a/SKILL.md", false).unwrap();
        set_disabled(&mut config, "/a/SKILL.md", false).unwrap();
        assert_eq!(config.disabled, ["/a/SKILL.md"]);
        set_disabled(&mut config, "/a/SKILL.md", true).unwrap();
        assert!(config.disabled.is_empty());
    }

    #[test]
    fn the_overview_marks_only_library_skills_editable_and_maps_package_switches() {
        let home = tempfile::tempdir().unwrap();
        let file = create_skill(home.path(), "mine", "Mine.", false, "").unwrap();
        let config = config(&[("claude", None, true)], &[&file.display().to_string()]);
        let listed = folders(&config, home.path());
        let package = PackageRecord {
            source: "npm:pkg".into(), display_name: "pkg".into(), kind: "npm".into(), version: None, installed_path: None,
            extensions: Vec::new(),
            skills: vec![PackageResourceRecord { path: "/pkg/skills/a".into(), name: "skills/a".into(), enabled: false }],
            prompts: Vec::new(), themes: Vec::new(), errors: Vec::new(), trusted_at: "t".into(), installed_at: "t".into(),
        };
        let skill = |name: &str, path: &str, resource: Option<&str>, shadowed: Option<&str>| ScannedSkill {
            name: name.into(), description: "d".into(), file_path: path.into(), base_dir: "/".into(), manual: false,
            resource_name: resource.map(str::to_string), shadowed_by: shadowed.map(str::to_string),
        };
        let scanned = ScanLine {
            ok: true,
            folders: vec![
                ScannedGroup { id: "library".into(), skills: vec![skill("mine", &file.display().to_string(), None, None)], diagnostics: Vec::new() },
                ScannedGroup { id: "claude".into(), skills: vec![skill("theirs", "/c/SKILL.md", None, Some("Your skills"))], diagnostics: vec![ScannedDiagnostic { kind: "warning".into(), message: "m".into(), path: None }] },
            ],
            packages: vec![ScannedGroup { id: "npm:pkg".into(), skills: vec![skill("a", "/pkg/skills/a/SKILL.md", Some("skills/a"), None)], diagnostics: Vec::new() }],
            error: None,
        };
        let overview = build_overview(home.path(), &config, &listed, &[&package], scanned);
        let library = &overview.folders[0];
        assert!(library.exists && library.enabled && library.skills[0].editable && !library.skills[0].enabled);
        assert_eq!(library.display_path, "~/.agents/skills");
        let claude = &overview.folders[1];
        assert!(!claude.exists && claude.enabled && !claude.skills[0].editable);
        assert_eq!(claude.skills[0].shadowed_by.as_deref(), Some("Your skills"));
        assert_eq!(claude.diagnostics.len(), 1);
        assert!(!overview.packages[0].skills[0].enabled && !overview.packages[0].skills[0].editable);
    }

    const CARD: &str = r#"<main class="content-shell packages-dashboard"><article class="surface-panel content-card" data-package-card="true" data-package-name="pi-gauntlet" data-package-search="x" data-package-types="extension skill" data-package-downloads="7973" data-package-date="1790208104095" data-package-sort-name="pi-gauntlet"><div class="packages-card-body"><h3 class="packages-name"><a href="/packages/pi-gauntlet">pi-gauntlet</a></h3><p class="packages-desc">Gated workflow skills &amp; personas for &quot;pi&quot; &#8212; fast</p><div class="packages-meta"><span>jjuraszek</span><span>7,973/mo</span><span>15h ago</span></div><div class="packages-links"><a href="https://github.com/earendil-works/pi/issues/new?template=package-report.yml&amp;package-name=pi-gauntlet&amp;package-version=5.18.5">report</a></div></div></article><article class="surface-panel content-card" data-package-card="true" data-package-name="@scope/skills" data-package-types="skill"><p class="packages-desc"></p></article><article class="surface-panel content-card" data-package-card="true" data-package-name="Not Valid"></article></main>"#;

    #[test]
    fn pidev_cards_are_read_from_their_data_attributes() {
        let results = parse_pidev(CARD).unwrap();
        assert_eq!(results.len(), 2);
        let first = &results[0];
        assert_eq!(first.name, "pi-gauntlet");
        assert_eq!(first.types, ["extension", "skill"]);
        assert_eq!(first.downloads, Some(7973));
        assert_eq!(first.version, "5.18.5");
        assert_eq!(first.publisher, "jjuraszek");
        assert_eq!(first.description, "Gated workflow skills & personas for \"pi\" \u{2014} fast");
        assert!(first.published_at.starts_with("2026-"));
        assert_eq!(first.npm_url, "https://www.npmjs.com/package/pi-gauntlet");
        assert_eq!(results[1].name, "@scope/skills");
        assert_eq!(results[1].description, "");
        assert_eq!(results[1].downloads, None);
    }

    #[test]
    fn an_empty_pidev_page_is_empty_but_a_changed_one_is_an_error() {
        assert!(parse_pidev(r#"<main class="packages-dashboard"><p class="packages-empty">No packages match this filter.</p></main>"#).unwrap().is_empty());
        assert!(parse_pidev("<html><body>Service unavailable</body></html>").is_err());
        assert!(parse_pidev(r#"<article data-package-card="true" data-renamed="x"></article><p class="packages-empty"></p>"#).is_err());
    }

    #[test]
    fn npm_names_are_checked() {
        for good in ["pi-gauntlet", "@scope/name", "a.b_c~d"] {
            assert!(valid_npm_name(good), "{good}");
        }
        for bad in ["", "Upper", "a b", "../x", "scope/name", "@scope", "<script>"] {
            assert!(!valid_npm_name(bad), "{bad}");
        }
    }

    #[test]
    fn entities_decode_without_eating_stray_ampersands() {
        assert_eq!(decode_entities("a &amp; b &lt;c&gt; &#39;d&#x27; &bogus; & e"), "a & b <c> 'd' &bogus; & e");
    }
}
