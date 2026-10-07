//! `/skill-creator`'s host side: managed draft workspaces, validation, and publication.
//!
//! Drafts live under `<app data>/agent/<task>/skill-creator/<draft-id>/` — private per chat,
//! never a skill root, cleaned up with the chat. Only the `skill/` subtree is ever published:
//! drafts and example runs never load as skills. Publication is a user action (the review
//! card's Save button) driven by `publish_skill_draft`; the agent itself never writes into
//! `~/.agents/skills`.
//!
//! Safety shape, in order:
//! 1. Ids from the renderer are strictly parsed (draft ids are UUIDs, revisions are SHA-256
//!    digests) before they ever reach a path join; every draft path is re-derived host-side.
//! 2. A save replays only the *reviewed* snapshot: the draft and the review copy must still
//!    hash to the reviewed revision, and an improving draft's baseline must be unchanged.
//! 3. Every library mutation (here and in Settings) shares `skills::library_guard`, held only
//!    around the check-commit-receipt window, never across a scan.
//! 4. Commits are journalled (`publication.json` in the draft). Folder updates swap atomically
//!    on macOS (`RENAME_SWAP`); the displaced original waits at a dot-prefixed, scan-invisible
//!    staging path until the receipt is durable, then is cleaned up. Interrupted commits are
//!    reconciled by hash — an outcome that cannot be proven is reported as unknown, never as
//!    saved.

use crate::models::{SkillDocument, SkillFolderKind};
use crate::skills::{self, Folder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    fs::Metadata,
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

/// Mirrors `skills::MAX_SKILL_FILE_BYTES` (kept private there).
const MAX_SKILL_FILE_BYTES: u64 = 1_000_000;
const MAX_BODY_CHARS: usize = 200_000;
const MAX_FILES: usize = 2_000;
const MAX_TOTAL_BYTES: u64 = 50 * 1024 * 1024;
const MAX_LISTED_FILES: usize = 200;
const MAX_BODY_PREVIEW_CHARS: usize = 16_000;
/// Directories a draft may never carry; `copy_tree` (Settings imports) skips these, a draft
/// rejects them so nothing silently disappears between preview and publish.
const FORBIDDEN_DIRECTORIES: &[&str] = &[".git", "node_modules"];

// ---------------------------------------------------------------------------------------------
// Result types (cross the bridge to the renderer)

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDraftStatus {
    pub draft_id: String,
    pub name: String,
    pub draft_root: String,
    /// drafting: no review yet · ready: a review the draft still matches · stale: the draft
    /// changed since its review · saved: published (see `path`) · unknown: an interrupted
    /// commit whose outcome could not be proven.
    pub state: String,
    pub revision: Option<String>,
    pub path: Option<String>,
    pub overwritten: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishSkillDraftResult {
    pub path: String,
    pub name: String,
    pub overwritten: bool,
}

// ---------------------------------------------------------------------------------------------
// Manifests and journal

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DraftManifest {
    v: u32,
    name: String,
    created_at: u64,
    #[serde(default)]
    origin: Option<DraftOrigin>,
    /// `hash_tree` of the origin at prepare time: the baseline a save re-checks.
    #[serde(default)]
    source_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct DraftOrigin {
    /// "library" (`~/.agents/skills`) or "external" (another tool's folder or a package).
    kind: String,
    /// The folder or package label the skill came from.
    label: String,
    /// The origin's SKILL.md (or loose `.md`) path.
    path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewManifest {
    v: u32,
    revision: String,
    created_at: u64,
    name: String,
    description: String,
    #[serde(default)]
    manual: bool,
    #[serde(default)]
    argument_hint: Option<String>,
    /// Display-only at review time; publish recomputes the real target.
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    origin_label: Option<String>,
    #[serde(default)]
    file_count: u32,
    #[serde(default)]
    total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicationJournal {
    v: u32,
    /// "committing" until the receipt is durable, then "saved".
    state: String,
    /// "new" (folder create), "swap" (folder update) or "file" (loose `.md` rewrite).
    kind: String,
    name: String,
    revision: String,
    /// The library path this publish targets.
    target: String,
    /// "swap"/"file": the hash the target held before the commit.
    #[serde(default)]
    baseline: Option<String>,
    /// "swap": where the displaced original waits until the receipt is durable.
    #[serde(default)]
    staging: Option<String>,
    #[serde(default)]
    saved_at: Option<u64>,
}

// ---------------------------------------------------------------------------------------------
// Paths and ids

fn app_data(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map_err(|_| "The app data folder could not be found.".to_string())
}

pub fn drafts_root(app: &AppHandle, task_id: &str) -> Result<PathBuf, String> {
    Ok(app_data(app)?.join("agent").join(task_id).join("skill-creator"))
}

fn validate_draft_id(draft_id: &str) -> Result<Uuid, String> {
    Uuid::parse_str(draft_id).map_err(|_| "That draft id is not valid.".to_string())
}

fn validate_revision(revision: &str) -> Result<(), String> {
    let valid = revision.len() == 64 && revision.chars().all(|character| character.is_ascii_hexdigit());
    if valid { Ok(()) } else { Err("That revision is not valid.".to_string()) }
}

fn draft_dir(app: &AppHandle, task_id: &str, draft_id: &str) -> Result<PathBuf, String> {
    let id = validate_draft_id(draft_id)?;
    Ok(drafts_root(app, task_id)?.join(id.as_simple().to_string()))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("Could not read {}: {error}", path.display()))
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|error| format!("Could not encode {}: {error}", path.display()))?;
    crate::skills::write_atomic(path, &text)
}

// ---------------------------------------------------------------------------------------------
// Hashing and strict copies

fn entry_kind(metadata: &Metadata) -> Result<&'static str, String> {
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err("The skill contains a symbolic link, which skills cannot carry. Replace it with the real file.".into());
    }
    if file_type.is_dir() { return Ok("dir"); }
    if metadata.is_file() { return Ok("file"); }
    Err("The skill contains a special file (a socket, device or FIFO), which skills cannot carry.".into())
}

/// One file's stable digest input: its path, its executable bit, then its bytes.
fn hash_file_into(hasher: &mut Sha256, root: &Path, relative: &Path, metadata: &Metadata) -> Result<(), String> {
    hasher.update(relative.to_string_lossy().as_bytes());
    hasher.update([0]);
    #[cfg(unix)]
    let executable = {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    };
    #[cfg(not(unix))]
    let executable = false;
    hasher.update([u8::from(executable)]);
    hasher.update([0]);
    let bytes = fs::read(root.join(relative)).map_err(|error| format!("Could not read {}: {error}", relative.display()))?;
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(&bytes);
    Ok(())
}

/// A content digest of a directory tree: sorted relative paths, executable bits and bytes.
/// Symlinks and special files are refused rather than skipped, so the hash a user reviewed is
/// provably the tree that gets published.
fn hash_tree(root: &Path) -> Result<String, String> {
    let mut relative_paths: Vec<PathBuf> = Vec::new();
    collect_files(root, Path::new(""), &mut relative_paths, &mut 0, &mut 0)?;
    relative_paths.sort();
    let mut hasher = Sha256::new();
    for relative in &relative_paths {
        let metadata = fs::symlink_metadata(root.join(relative)).map_err(|error| format!("Could not read {}: {error}", relative.display()))?;
        hash_file_into(&mut hasher, root, relative, &metadata)?;
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn hash_single_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&(bytes.len() as u64).to_be_bytes());
    hasher.update(&bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

/// Every regular file under `root`, relative — the same walk (and the same limits) `hash_tree`
/// and the copy use, so validation, hashing and publishing all see the same tree.
fn collect_files(root: &Path, relative: &Path, out: &mut Vec<PathBuf>, files: &mut usize, bytes: &mut u64) -> Result<(), String> {
    let directory = root.join(relative);
    let entries = fs::read_dir(&directory).map_err(|error| format!("Could not read {}: {error}", directory.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let metadata = entry.metadata().map_err(|error| format!("Could not read {}/{}: {error}", directory.display(), name.to_string_lossy()))?;
        let kind = entry_kind(&metadata)?;
        let child = relative.join(&name);
        if kind == "dir" {
            if FORBIDDEN_DIRECTORIES.contains(&name.to_string_lossy().as_ref()) {
                return Err(format!("The skill cannot contain a {name:?} folder; remove it from the draft."));
            }
            collect_files(root, &child, out, files, bytes)?;
        } else {
            *files += 1;
            *bytes += metadata.len();
            if *files > MAX_FILES || *bytes > MAX_TOTAL_BYTES {
                return Err(format!(
                    "That skill is too large (at most {MAX_FILES} files and {} MB).",
                    MAX_TOTAL_BYTES / 1024 / 1024
                ));
            }
            out.push(child);
        }
    }
    Ok(())
}

/// Copy a whole tree preserving permission bits, refusing symlinks and special files.
fn copy_tree_strict(from: &Path, to: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    let mut count = 0;
    let mut bytes = 0;
    collect_files(from, Path::new(""), &mut files, &mut count, &mut bytes)?;
    fs::create_dir_all(to).map_err(|error| format!("Could not create {}: {error}", to.display()))?;
    for relative in &files {
        let target = to.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
        }
        fs::copy(from.join(relative), &target).map_err(|error| format!("Could not copy {}: {error}", relative.display()))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Prepare

/// The roots an existing skill may be read from: every listed skill folder (Settings › Skills)
/// and every trusted package's install path — the same set `read_skill` allows.
fn skill_roots(app: &AppHandle, state: &State<'_, crate::storage::MetadataState>) -> Result<Vec<PathBuf>, String> {
    let home = skills::home_dir(app)?;
    let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
    Ok(skills::folders(&data.skills, &home)
        .into_iter()
        .map(|folder| folder.path)
        .chain(
            data.packages
                .iter()
                .filter(|package| !package.trusted_at.is_empty())
                .filter_map(|package| package.installed_path.as_deref().map(PathBuf::from)),
        )
        .collect())
}

/// Create (or reattach to) the managed draft for `name`, snapshotting `source_path` when
/// improving an existing skill. The agent authors inside the returned paths with its ordinary
/// file tools; nothing here touches the shared library.
pub fn prepare(
    app: &AppHandle,
    state: &State<'_, crate::storage::MetadataState>,
    task_id: &str,
    name: &str,
    source_path: Option<&str>,
) -> Result<Value, String> {
    let name = skills::validate_name(name)?;
    let home = skills::home_dir(app)?;
    let library = skills::library_dir(&home);

    let origin: Option<DraftOrigin> = match source_path {
        Some(path) => {
            let roots = skill_roots(app, state)?;
            let file = skills::readable_skill(path, &roots)?;
            let kind = skills::inside(&file, &library).is_some();
            let label = folder_label(&file, &home, state)?;
            Some(DraftOrigin {
                kind: if kind { "library".into() } else { "external".into() },
                label,
                path: file.display().to_string(),
            })
        }
        None => None,
    };

    // A name may only shadow an existing library entry when this draft *is* that entry (an
    // update of the same folder or loose file); the cheap structural check here, Pi's own
    // parse at publish.
    let updates_itself = origin.as_ref().is_some_and(|origin| {
        Path::new(&origin.path) == library.join(&name).join("SKILL.md")
            || Path::new(&origin.path) == library.join(format!("{name}.md"))
    });
    if !updates_itself && (library.join(&name).exists() || library.join(format!("{name}.md")).exists()) {
        return Err(format!("A skill named {name} already exists in Your skills."));
    }

    let root = drafts_root(app, task_id)?;
    fs::create_dir_all(&root).map_err(|error| format!("Could not create the draft workspace: {error}"))?;

    // Reattach: a draft for this name with no publication yet keeps its work and examples.
    if let Some(existing) = latest_draft_for(&root, &name) {
        let manifest: DraftManifest = read_json(&existing.join("manifest.json"))?;
        if manifest.origin == origin {
            return Ok(json!({
                "draftId": existing.file_name().map(|id| id.to_string_lossy().into_owned()).unwrap_or_default(),
                "draftRoot": existing.display().to_string(),
                "skillDir": existing.join("skill").display().to_string(),
                "evalsDir": existing.join("evals").display().to_string(),
                "originalDir": manifest.origin.as_ref().map(|_| existing.join("original").display().to_string()),
                "name": manifest.name,
                "originLabel": manifest.origin.as_ref().map(|origin| origin.label.clone()),
            }));
        }
    }

    let draft_root = root.join(Uuid::new_v4().to_string());
    let skill_dir = draft_root.join("skill");
    let evals_dir = draft_root.join("evals");
    let original_dir = draft_root.join("original");
    fs::create_dir(&draft_root).map_err(|error| format!("Could not create the draft workspace: {error}"))?;
    fs::create_dir(&skill_dir).map_err(|error| format!("Could not create the draft workspace: {error}"))?;
    fs::create_dir(&evals_dir).map_err(|error| format!("Could not create the draft workspace: {error}"))?;

    // Snapshot the origin before any authoring: the agent reads and improves the copy, and the
    // recorded baseline is what a save re-checks the live skill against.
    let source_hash = match &origin {
        Some(draft_origin) => {
            let file = Path::new(&draft_origin.path);
            fs::create_dir(&original_dir).map_err(|error| format!("Could not create the draft workspace: {error}"))?;
            if file.file_name().is_some_and(|name| name == "SKILL.md") {
                let base = file.parent().ok_or("That skill has no folder.")?;
                copy_tree_strict(base, &original_dir)?;
            } else {
                fs::copy(file, original_dir.join(file.file_name().unwrap_or_default()))
                    .map_err(|error| format!("Could not copy the skill: {error}"))?;
            }
            Some(hash_tree(&original_dir)?)
        }
        None => None,
    };

    let manifest = DraftManifest {
        v: 1,
        name: name.clone(),
        created_at: now_ms(),
        origin: origin.clone(),
        source_hash,
    };
    write_json_atomic(&draft_root.join("manifest.json"), &manifest)?;

    Ok(json!({
        "draftId": draft_root.file_name().map(|id| id.to_string_lossy().into_owned()).unwrap_or_default(),
        "draftRoot": draft_root.display().to_string(),
        "skillDir": skill_dir.display().to_string(),
        "evalsDir": evals_dir.display().to_string(),
        "originalDir": if origin.is_some() { Value::String(original_dir.display().to_string()) } else { Value::Null },
        "name": name,
        "originLabel": origin.map(|origin| origin.label),
    }))
}

/// The newest draft for `name` that has not been published yet.
fn latest_draft_for(root: &Path, name: &str) -> Option<PathBuf> {
    let mut candidates: Vec<(u64, PathBuf)> = Vec::new();
    for entry in fs::read_dir(root).ok()?.flatten() {
        let path = entry.path();
        if !path.is_dir() || Uuid::parse_str(&entry.file_name().to_string_lossy()).is_err() {
            continue;
        }
        let Ok(manifest): Result<DraftManifest, _> = read_json(&path.join("manifest.json")) else { continue };
        if manifest.name != name || path.join("publication.json").exists() {
            continue;
        }
        candidates.push((manifest.created_at, path));
    }
    candidates.sort();
    candidates.pop().map(|(_, path)| path)
}

/// The label Settings would show for a skill at this path: its folder's label, or the package's.
fn folder_label(
    file: &Path,
    home: &Path,
    state: &State<'_, crate::storage::MetadataState>,
) -> Result<String, String> {
    let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
    for folder in skills::folders(&data.skills, home) {
        if file.starts_with(&folder.path) {
            return Ok(folder.label);
        }
    }
    for package in &data.packages {
        if package.trusted_at.is_empty() {
            continue;
        }
        if let Some(installed) = package.installed_path.as_deref() {
            if file.starts_with(Path::new(installed)) {
                return Ok(package.display_name.clone());
            }
        }
    }
    Ok("Another folder".into())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|duration| duration.as_millis() as u64).unwrap_or(0)
}

// ---------------------------------------------------------------------------------------------
// Preview

/// Where a draft with this origin and name would publish, and whether that overwrites.
#[derive(Debug, Clone, PartialEq)]
enum Target {
    /// A new folder in the library (also the shape for copies from other folders/packages).
    New,
    /// Replace a standard `~/.agents/skills/<name>/` folder.
    SwapFolder(PathBuf),
    /// Rewrite a loose `~/.agents/skills/<name>.md` file in place (single-file drafts only).
    LooseFile(PathBuf),
}

fn resolve_target(library: &Path, origin: &Option<DraftOrigin>, name: &str, files_only_skill_md: bool, origin_name: &str) -> Result<Target, String> {
    let Some(origin) = origin else { return Ok(Target::New) };
    if origin.kind != "library" {
        return Ok(Target::New);
    }
    let path = Path::new(&origin.path);
    if path.file_name().is_some_and(|file| file == "SKILL.md") {
        if let Some(folder) = path.parent() {
            if folder.parent() == Some(library) && folder.file_name().is_some_and(|dir| dir == name) {
                return Ok(Target::SwapFolder(folder.to_path_buf()));
            }
            // A library skill that is not a plain top-level folder (nested or renamed) publishes
            // as a separately named copy; its original is left alone.
            return Ok(Target::New);
        }
    }
    // Loose `.md` in the library: rewrite it in place only while the draft stays single-file
    // and keeps the skill's name; anything richer becomes a proper folder with a free name.
    if path.parent() == Some(library) && files_only_skill_md && name == origin_name {
        return Ok(Target::LooseFile(path.to_path_buf()));
    }
    Ok(Target::New)
}

/// Validate the draft with Pi's own loader (the same parse a chat will do), hash it, and write
/// the immutable review snapshot the Save button later publishes.
pub async fn preview(app: &AppHandle, task_id: &str, draft_id: &str) -> Result<Value, String> {
    let draft_root = draft_dir(app, task_id, draft_id)?;
    let skill_dir = draft_root.join("skill");
    let manifest: DraftManifest = read_json(&draft_root.join("manifest.json"))
        .map_err(|_| "That draft is no longer available. Run /skill-creator again.".to_string())?;
    if !skill_dir.join("SKILL.md").is_file() {
        return Err("The draft has no SKILL.md yet. Write the skill first, then preview.".into());
    }

    // The strict walk first: limits, symlinks, special files and forbidden folders.
    let mut files = Vec::new();
    let mut file_count = 0usize;
    let mut total_bytes = 0u64;
    collect_files(&skill_dir, Path::new(""), &mut files, &mut file_count, &mut total_bytes)?;
    let skill_file_metadata = fs::metadata(skill_dir.join("SKILL.md")).map_err(|error| format!("Could not read the draft's SKILL.md: {error}"))?;
    if skill_file_metadata.len() > MAX_SKILL_FILE_BYTES {
        return Err("The draft's SKILL.md is larger than 1 MB.".into());
    }

    // Pi's own parse: exactly one skill, discovered at the draft's SKILL.md.
    let folder = Folder {
        id: "draft".into(),
        label: "Draft".into(),
        path: skill_dir.clone(),
        kind: SkillFolderKind::Custom,
        enabled: true,
    };
    let (scanned, _diagnostics) = skills::scan_single_folder(app, &folder).await?;
    let skill = scanned
        .iter()
        .find(|skill| Path::new(&skill.file_path) == skill_dir.join("SKILL.md"))
        .ok_or_else(|| {
            if scanned.is_empty() {
                "The draft's SKILL.md could not be parsed. It needs `name` and `description` frontmatter.".to_string()
            } else {
                "The draft must hold exactly one skill: move nested SKILL.md folders out of it.".to_string()
            }
        })?
        .clone();
    if scanned.len() > 1 {
        return Err("The draft must hold exactly one skill: move nested SKILL.md folders out of it.".into());
    }
    let name = skills::validate_name(&skill.name)?;
    let description = skills::validate_description(&skill.description)?;
    let hint = skill.argument_hint.as_deref().map(str::trim).filter(|value| !value.is_empty()).map(str::to_string).unwrap_or_default();
    skills::validate_hint(&hint)?;

    let text = fs::read_to_string(skill_dir.join("SKILL.md")).map_err(|error| format!("Could not read the draft's SKILL.md: {error}"))?;
    let (_, body) = skills::split(&text);
    if body.trim().is_empty() {
        return Err("The skill needs instructions below its frontmatter.".into());
    }
    if body.chars().count() > MAX_BODY_CHARS {
        return Err(format!("A skill's instructions can be at most {MAX_BODY_CHARS} characters."));
    }

    let revision = hash_tree(&skill_dir)?;
    let review_dir = draft_root.join("review").join(&revision);
    if review_dir.exists() {
        fs::remove_dir_all(&review_dir).map_err(|error| format!("Could not refresh the review snapshot: {error}"))?;
    }
    copy_tree_strict(&skill_dir, &review_dir)?;

    let files_only_skill_md = file_count == 1;
    let target = resolve_target(&skills::library_dir(&skills::home_dir(app)?), &manifest.origin, &name, files_only_skill_md, &manifest.name)?;
    let target_label = match target {
        Target::New => {
            if manifest.origin.is_some() { "library-copy" } else { "new" }
        }
        _ => "library-update",
    };
    let mut warnings = Vec::new();
    if name != manifest.name {
        warnings.push(format!("The skill's name ({name}) differs from the prepared name ({}).", manifest.name));
    }
    let executable = files.iter().filter(|file| {
        fs::metadata(skill_dir.join(file)).map(|metadata| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            { false }
        }).unwrap_or(false)
    }).count();
    if executable > 0 {
        warnings.push(format!("{executable} executable file{} copied with their permissions.", if executable == 1 { " is" } else { "s are" }));
    }

    let review_manifest = ReviewManifest {
        v: 1,
        revision: revision.clone(),
        created_at: now_ms(),
        name: name.clone(),
        description: description.clone(),
        manual: skill.manual,
        argument_hint: if hint.is_empty() { None } else { Some(hint.clone()) },
        target: Some(target_label.to_string()),
        origin_label: manifest.origin.as_ref().map(|origin| origin.label.clone()),
        file_count: file_count as u32,
        total_bytes,
    };
    write_json_atomic(&draft_root.join("review").join(format!("{revision}.json")), &review_manifest)?;

    let body_chars: Vec<char> = body.chars().collect();
    let body_preview: String = body_chars.iter().take(MAX_BODY_PREVIEW_CHARS).collect();
    let body_truncated = body_chars.len() > MAX_BODY_PREVIEW_CHARS;
    let supporting: Vec<String> = files
        .iter()
        .map(|file| file.display().to_string())
        .filter(|file| file != "SKILL.md")
        .take(MAX_LISTED_FILES)
        .collect();

    Ok(json!({
        "draftId": draft_id,
        "revision": revision,
        "name": name,
        "description": description,
        "manual": skill.manual,
        "argumentHint": if hint.is_empty() { Value::Null } else { json!(hint) },
        "target": target_label,
        "originLabel": manifest.origin.as_ref().map(|origin| origin.label.clone()),
        "bodyPreview": body_preview,
        "bodyTruncated": body_truncated,
        "files": supporting,
        "fileCount": file_count,
        "totalBytes": total_bytes,
        "warnings": warnings,
    }))
}

// ---------------------------------------------------------------------------------------------
// Publish

#[derive(Debug)]
struct PublishOutcome {
    path: PathBuf,
    name: String,
    overwritten: bool,
}

fn commit_new_folder(library: &Path, draft_root: &Path, review_dir: &Path, journal: &PublicationJournal) -> Result<PathBuf, String> {
    let target = library.join(&journal.name);
    if target.exists() || library.join(format!("{}.md", journal.name)).exists() {
        return Err(format!("A skill named {} already exists in Your skills.", journal.name));
    }
    let staging = library.join(format!(".wackcode-skillpub-{}", Uuid::new_v4().as_simple()));
    copy_tree_strict(review_dir, &staging)?;
    write_json_atomic(&staging.join(".wackcode-skillpub.json"), &json!({
        "draftRoot": draft_root.display().to_string(),
        "revision": journal.revision,
    }))?;
    let commit = commit_exclusive(&staging, &target, &journal.name);
    if commit.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    commit?;
    Ok(target)
}

/// macOS-exclusive rename: the commit itself refuses to clobber, closing the gap between the
/// name check and the rename even if something else creates the destination meanwhile.
fn commit_exclusive(staging: &Path, target: &Path, name: &str) -> Result<(), String> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let from = CString::new(staging.as_os_str().as_bytes()).map_err(|_| "Invalid staging path.".to_string())?;
    let to = CString::new(target.as_os_str().as_bytes()).map_err(|_| "Invalid skill path.".to_string())?;
    // SAFETY: both paths are NUL-terminated and remain alive throughout this call.
    let result = unsafe { nix::libc::renamex_np(from.as_ptr(), to.as_ptr(), nix::libc::RENAME_EXCL) };
    if result == 0 { return Ok(()); }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        Err(format!("A skill named {name} already exists in Your skills."))
    } else {
        Err(format!("Could not save {name}: {error}"))
    }
}

/// macOS swap: after this call `target` holds the new skill and `staging` holds the displaced
/// original, waiting at a dot-prefixed path no skill scan reads.
fn swap_folders(staging: &Path, target: &Path, name: &str) -> Result<(), String> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let from = CString::new(staging.as_os_str().as_bytes()).map_err(|_| "Invalid staging path.".to_string())?;
    let to = CString::new(target.as_os_str().as_bytes()).map_err(|_| "Invalid skill path.".to_string())?;
    // SAFETY: both paths are NUL-terminated and remain alive throughout this call.
    let result = unsafe { nix::libc::renamex_np(from.as_ptr(), to.as_ptr(), nix::libc::RENAME_SWAP) };
    if result == 0 { return Ok(()); }
    Err(format!("Could not save {name}: {}", std::io::Error::last_os_error()))
}

fn journal_path(draft_root: &Path) -> PathBuf {
    draft_root.join("publication.json")
}

fn write_journal(draft_root: &Path, journal: &PublicationJournal) -> Result<(), String> {
    write_json_atomic(&journal_path(draft_root), journal)
}

/// The library mutation itself. The caller holds `skills::library_guard` and has already
/// re-verified the draft against its review.
fn do_publish(
    library: &Path,
    draft_root: &Path,
    revision: &str,
    manifest: &DraftManifest,
    review: &ReviewManifest,
) -> Result<PublishOutcome, String> {
    let skill_dir = draft_root.join("skill");
    let review_dir = draft_root.join("review").join(revision);
    let name = skills::validate_name(&review.name)?;

    // Only the reviewed snapshot is ever published, and only while it still matches the draft.
    if hash_tree(&skill_dir)? != revision {
        return Err("The draft changed since you reviewed it. Ask for a new preview, then save.".into());
    }
    if hash_tree(&review_dir)? != revision {
        return Err("The reviewed copy no longer matches its revision. Preview the draft again, then save.".into());
    }
    let files_only_skill_md = review.file_count <= 1;
    let target = resolve_target(library, &manifest.origin, &name, files_only_skill_md, &manifest.name)?;

    // A save whose origin moved on under the user refuses rather than clobbering.
    let baseline = manifest.source_hash.clone();
    if let Some(expected) = baseline.as_deref() {
        match &target {
            Target::SwapFolder(folder) => {
                if hash_tree(folder)? != expected {
                    return Err("That skill changed on disk since this draft started. Preview it again from its current version, then save.".into());
                }
            }
            Target::LooseFile(file) => {
                if hash_single_file(file)? != expected {
                    return Err("That skill changed on disk since this draft started. Preview it again from its current version, then save.".into());
                }
            }
            Target::New => {}
        }
    }

    match &target {
        Target::New => {
            let journal = PublicationJournal {
                v: 1,
                state: "committing".into(),
                kind: "new".into(),
                name: name.clone(),
                revision: revision.to_string(),
                target: library.join(&name).display().to_string(),
                baseline: None,
                staging: None,
                saved_at: None,
            };
            write_journal(draft_root, &journal)?;
            let target_path = commit_new_folder(library, draft_root, &review_dir, &journal)?;
            let mut receipt = journal;
            receipt.state = "saved".into();
            receipt.saved_at = Some(now_ms());
            write_journal(draft_root, &receipt)?;
            Ok(PublishOutcome { path: target_path, name, overwritten: false })
        }
        Target::SwapFolder(folder) => {
            let staging = library.join(format!(".wackcode-skillpub-{}", Uuid::new_v4().as_simple()));
            copy_tree_strict(&review_dir, &staging)?;
            let journal = PublicationJournal {
                v: 1,
                state: "committing".into(),
                kind: "swap".into(),
                name: name.clone(),
                revision: revision.to_string(),
                target: folder.display().to_string(),
                baseline,
                staging: Some(staging.display().to_string()),
                saved_at: None,
            };
            write_journal(draft_root, &journal)?;
            let swapped = swap_folders(&staging, folder, &name).and_then(|_| {
                if hash_tree(folder)? != revision {
                    // Verified failure: put the original back before reporting.
                    let _ = swap_folders(&staging, folder, &name);
                    Err("The saved skill did not match its review. Nothing was changed; try again.".to_string())
                } else {
                    Ok(())
                }
            });
            if let Err(error) = swapped {
                let _ = fs::remove_dir_all(&staging);
                return Err(error);
            }
            let mut receipt = journal;
            receipt.state = "saved".into();
            receipt.saved_at = Some(now_ms());
            write_journal(draft_root, &receipt)?;
            // The receipt is durable: the displaced original can go.
            let _ = fs::remove_dir_all(&staging);
            Ok(PublishOutcome { path: folder.clone(), name, overwritten: true })
        }
        Target::LooseFile(file) => {
            let journal = PublicationJournal {
                v: 1,
                state: "committing".into(),
                kind: "file".into(),
                name: name.clone(),
                revision: revision.to_string(),
                target: file.display().to_string(),
                baseline,
                staging: None,
                saved_at: None,
            };
            write_journal(draft_root, &journal)?;
            let reviewed = fs::read(review_dir.join("SKILL.md")).map_err(|error| format!("Could not read the reviewed skill: {error}"))?;
            crate::skills::write_atomic(file, &String::from_utf8_lossy(&reviewed))?;
            if fs::read(file).unwrap_or_default() != reviewed {
                return Err("The saved skill did not match its review. Try previewing and saving again.".into());
            }
            let mut receipt = journal;
            receipt.state = "saved".into();
            receipt.saved_at = Some(now_ms());
            write_journal(draft_root, &receipt)?;
            Ok(PublishOutcome { path: file.clone(), name, overwritten: true })
        }
    }
}

/// Publish a reviewed revision into the shared library. The task lock, idle/Build checks and
/// the pre-lock name scan happen in the command; everything after `library_guard` is here.
pub async fn publish(
    app: &AppHandle,
    task_id: &str,
    draft_id: &str,
    revision: &str,
) -> Result<PublishSkillDraftResult, String> {
    validate_revision(revision)?;
    let draft_root = draft_dir(app, task_id, draft_id)?;
    let manifest: DraftManifest = read_json(&draft_root.join("manifest.json"))
        .map_err(|_| "That draft is no longer available. Run /skill-creator again.".to_string())?;
    let review_path = draft_root.join("review").join(format!("{revision}.json"));
    let review: ReviewManifest = read_json(&review_path)
        .map_err(|_| "That review is no longer available. Preview the draft again, then save.".to_string())?;
    if review.revision != revision {
        return Err("That review is no longer available. Preview the draft again, then save.".into());
    }

    let home = skills::home_dir(app)?;
    let library = skills::library_dir(&home);

    // Before the lock: where this revision would land, and Pi's own parse of the library when
    // it is a new (or copied) name, so a same-named skill at another path (a loose `.md`, a
    // nested folder) can never be silently shadowed by this publish.
    let name = skills::validate_name(&review.name)?;
    let files_only_skill_md = review.file_count <= 1;
    if resolve_target(&library, &manifest.origin, &name, files_only_skill_md, &manifest.name)? == Target::New {
        let folder = Folder {
            id: "library".into(),
            label: "Your skills".into(),
            path: library.clone(),
            kind: SkillFolderKind::Library,
            enabled: true,
        };
        if let Ok((scanned, _)) = skills::scan_single_folder(app, &folder).await {
            if let Some(existing) = scanned.iter().find(|skill| skill.name == name && Path::new(&skill.file_path) != library.join(&name).join("SKILL.md")) {
                return Err(format!(
                    "A skill named {name} already exists in Your skills ({}). Choose another name, or remove it first.",
                    existing.file_path
                ));
            }
        }
    }

    let _guard = skills::library_guard().await;
    let outcome = do_publish(&library, &draft_root, revision, &manifest, &review)?;
    Ok(PublishSkillDraftResult {
        path: outcome.path.display().to_string(),
        name: outcome.name,
        overwritten: outcome.overwritten,
    })
}

// ---------------------------------------------------------------------------------------------
// Status

/// Reconcile an interrupted commit by hash. Returns the durable journal when the publish is
/// proven saved, cleans up when it provably never happened, and leaves an unprovable state
/// reported as "unknown" rather than guessing.
fn reconcile_journal(draft_root: &Path, journal: &PublicationJournal) -> Result<Option<PublicationJournal>, String> {
    if journal.state == "saved" {
        return Ok(Some(journal.clone()));
    }
    let target = Path::new(&journal.target);
    if journal.kind == "file" {
        // A loose-file publish is proven by its bytes against the reviewed snapshot.
        let reviewed = fs::read(draft_root.join("review").join(&journal.revision).join("SKILL.md"));
        let written = fs::read(target);
        if let (Ok(reviewed), Ok(written)) = (reviewed, written) {
            if reviewed == written {
                let mut receipt = journal.clone();
                receipt.state = "saved".into();
                receipt.saved_at = Some(now_ms());
                write_journal(draft_root, &receipt)?;
                return Ok(Some(receipt));
            }
            if journal.baseline.as_deref().is_some_and(|baseline| hash_single_file(target).as_deref() == Ok(baseline)) {
                let _ = fs::remove_file(journal_path(draft_root));
                return Ok(None);
            }
        }
        return Ok(Some(journal.clone())); // committing, unproven
    }
    let target_hash = if target.is_dir() { Some(hash_tree(target)?) } else { None };
    match target_hash.as_deref() {
        Some(hash) if hash == journal.revision => {
            let mut receipt = journal.clone();
            receipt.state = "saved".into();
            receipt.saved_at = Some(now_ms());
            write_journal(draft_root, &receipt)?;
            if let Some(staging) = journal.staging.as_deref() {
                let _ = fs::remove_dir_all(Path::new(staging));
            }
            Ok(Some(receipt))
        }
        // The target still holds what it held before: the commit never landed.
        Some(hash) if journal.kind != "new" && Some(hash) == journal.baseline.as_deref() => {
            let _ = fs::remove_file(journal_path(draft_root));
            if let Some(staging) = journal.staging.as_deref() {
                let _ = fs::remove_dir_all(Path::new(staging));
            }
            Ok(None)
        }
        None if journal.kind == "new" => {
            let _ = fs::remove_file(journal_path(draft_root));
            Ok(None)
        }
        _ => Ok(Some(journal.clone())), // committing, unproven
    }
}

/// Every draft this chat owns, with its latest review and publication state, for the review
/// cards' hydration. Read-only apart from interrupted-commit reconciliation.
pub fn status(app: &AppHandle, task_id: &str) -> Result<Vec<SkillDraftStatus>, String> {
    let root = drafts_root(app, task_id)?;
    let mut out = Vec::new();
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(_) => return Ok(out),
    };
    for entry in entries.flatten() {
        let draft_root = entry.path();
        if !draft_root.is_dir() || validate_draft_id(&entry.file_name().to_string_lossy()).is_err() {
            continue;
        }
        let Ok(manifest): Result<DraftManifest, _> = read_json(&draft_root.join("manifest.json")) else { continue };

        // The latest review by its manifest's creation time.
        let mut latest: Option<(u64, ReviewManifest)> = None;
        if let Ok(reviews) = fs::read_dir(draft_root.join("review")) {
            for review in reviews.flatten() {
                let path = review.path();
                if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                    continue;
                }
                if let Ok(review_manifest) = read_json::<ReviewManifest>(&path) {
                    if latest.as_ref().map(|(at, _)| review_manifest.created_at > *at).unwrap_or(true) {
                        latest = Some((review_manifest.created_at, review_manifest));
                    }
                }
            }
        }

        let journal: Option<PublicationJournal> = read_json(&journal_path(&draft_root)).ok();
        let journal = match journal {
            Some(journal) => reconcile_journal(&draft_root, &journal)?,
            None => None,
        };

        let revision = latest.as_ref().map(|(_, review)| review.revision.clone());
        let state = if let Some(journal) = &journal {
            if journal.state == "saved" { "saved" } else { "unknown" }
        } else if let Some((_, review)) = &latest {
            let current = hash_tree(&draft_root.join("skill"));
            match current {
                Ok(hash) if hash == review.revision => "ready",
                Ok(_) => "stale",
                Err(_) => "unknown",
            }
        } else {
            "drafting"
        };

        out.push(SkillDraftStatus {
            draft_id: entry.file_name().to_string_lossy().into_owned(),
            name: manifest.name,
            draft_root: draft_root.display().to_string(),
            state: state.to_string(),
            revision,
            path: journal.as_ref().map(|journal| journal.target.clone()),
            overwritten: journal.as_ref().map(|journal| journal.kind != "new").unwrap_or(false),
        });
    }
    out.sort_by(|a, b| b.draft_root.cmp(&a.draft_root));
    Ok(out)
}

/// The draft's current SKILL.md document, for the card's full-instructions view.
pub fn read_document(app: &AppHandle, task_id: &str, draft_id: &str) -> Result<SkillDocument, String> {
    let draft_root = draft_dir(app, task_id, draft_id)?;
    let skill_file = draft_root.join("skill").join("SKILL.md");
    if !skill_file.is_file() {
        return Err("That draft has no SKILL.md.".into());
    }
    skills::read_document(&skill_file)
}

// ---------------------------------------------------------------------------------------------
// Worker bridge

/// `skill_creator_request` from the worker: prepare or preview. Never starts a model, never
/// touches the shared library; both operations stay inside the chat's managed workspace.
pub async fn execute_agent_request(app: &AppHandle, task_id: &str, request: Value) -> Result<Value, String> {
    let state = app.state::<crate::storage::MetadataState>();
    let operation = request.get("op").and_then(Value::as_str).unwrap_or("");
    match operation {
        "prepare" => {
            let name = request.get("name").and_then(Value::as_str).ok_or("The skill's name is required.")?;
            let source_path = request.get("sourcePath").and_then(Value::as_str);
            let result = prepare(app, &state, task_id, name, source_path)?;
            Ok(result)
        }
        "preview" => {
            let draft_id = request.get("draftId").and_then(Value::as_str).ok_or("The draft id is required.")?;
            preview(app, task_id, draft_id).await
        }
        _ => Err("Unknown skill-creator operation.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        tempfile::tempdir().expect("tempdir").keep().join(name)
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().expect("parent")).expect("create");
        fs::write(path, text).expect("write");
    }

    #[test]
    fn draft_and_revision_ids_are_strictly_validated() {
        assert!(validate_draft_id("0f14d0ab-9605-4a62-a9e4-5ed26688389b").is_ok());
        assert!(validate_draft_id("../../etc").is_err());
        assert!(validate_draft_id("").is_err());
        assert!(validate_revision(&"a".repeat(64)).is_ok());
        assert!(validate_revision(&"a".repeat(63)).is_err());
        assert!(validate_revision(&"z".repeat(64)).is_err());
        assert!(validate_revision("../../../../etc/passwd").is_err());
    }

    #[test]
    fn hash_tree_is_content_addressed_over_paths_exec_bits_and_bytes() {
        let root = temp("hash");
        write(&root.join("SKILL.md"), "hello");
        write(&root.join("scripts/run.sh"), "#!/bin/sh\n");
        let hash = hash_tree(&root).expect("hash");
        // Same tree, same hash.
        assert_eq!(hash_tree(&root).unwrap(), hash);
        // Different content, different hash.
        write(&root.join("SKILL.md"), "hello!");
        assert_ne!(hash_tree(&root).unwrap(), hash);
        // Executable bit participates.
        let before = hash_tree(&root).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root.join("scripts/run.sh"), fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        assert_ne!(hash_tree(&root).unwrap(), before);
    }

    #[test]
    fn collect_files_rejects_symlinks_forbidden_folders_and_oversized_trees() {
        let root = temp("strict");
        write(&root.join("SKILL.md"), "body");
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("SKILL.md"), root.join("link.md")).expect("symlink");
        assert!(hash_tree(&root).is_err());

        let clean = temp("clean");
        write(&clean.join("SKILL.md"), "body");
        assert!(hash_tree(&clean).is_ok());
        write(&clean.join(".git/config"), "");
        assert!(collect_files(&clean, Path::new(""), &mut Vec::new(), &mut 0, &mut 0).is_err());
    }

    fn draft_with_skill(name: &str, body: &str, origin: Option<DraftOrigin>, source_hash: Option<String>) -> (PathBuf, DraftManifest, ReviewManifest) {
        let draft_root = temp("draft");
        write(&draft_root.join("skill/SKILL.md"), &format!("---\nname: {name}\ndescription: d\n---\n\n{body}"));
        let manifest = DraftManifest { v: 1, name: name.to_string(), created_at: 1, origin, source_hash };
        write_json_atomic(&draft_root.join("manifest.json"), &manifest).expect("manifest");
        let revision = hash_tree(&draft_root.join("skill")).unwrap();
        let review_dir = draft_root.join("review").join(&revision);
        copy_tree_strict(&draft_root.join("skill"), &review_dir).expect("review");
        let review = ReviewManifest {
            v: 1,
            revision: revision.clone(),
            created_at: 2,
            name: name.to_string(),
            description: "d".into(),
            manual: false,
            argument_hint: None,
            target: Some("new".into()),
            origin_label: None,
            file_count: 1,
            total_bytes: 8,
        };
        write_json_atomic(&draft_root.join("review").join(format!("{revision}.json")), &review).expect("review manifest");
        (draft_root, manifest, review)
    }

    #[test]
    fn publish_new_skill_commits_exclusively_and_writes_a_receipt() {
        let library = temp("library");
        fs::create_dir_all(&library).expect("create");
        let (draft_root, manifest, review) = draft_with_skill("fresh", "body", None, None);
        let outcome = do_publish(&library, &draft_root, &review.revision, &manifest, &review).expect("publish");
        assert_eq!(outcome.name, "fresh");
        assert!(!outcome.overwritten);
        assert!(library.join("fresh/SKILL.md").is_file());
        let journal: PublicationJournal = read_json(&journal_path(&draft_root)).expect("journal");
        assert_eq!(journal.state, "saved");
        // No staging remains behind.
        assert!(!library.join(".wackcode-skillpub-x").exists());
        let leftovers: Vec<_> = fs::read_dir(&library).expect("read").flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("."))
            .collect();
        assert!(leftovers.is_empty());
        // A second publish of the same name is refused.
        let (other_root, other_manifest, other_review) = draft_with_skill("fresh", "other", None, None);
        assert!(do_publish(&library, &other_root, &other_review.revision, &other_manifest, &other_review).is_err());
    }

    #[test]
    fn publish_refuses_when_the_draft_or_review_moved_on() {
        let library = temp("library2");
        let (draft_root, manifest, review) = draft_with_skill("moved", "body", None, None);
        // The draft changed after the review.
        write(&draft_root.join("skill/SKILL.md"), "---\nname: moved\ndescription: d\n---\n\nedited");
        let error = do_publish(&library, &draft_root, &review.revision, &manifest, &review).unwrap_err();
        assert!(error.contains("changed since you reviewed it"));
    }

    #[test]
    fn publish_swaps_a_library_folder_and_recovers_an_interrupted_commit() {
        let library = temp("library3");
        write(&library.join("old/SKILL.md"), "---\nname: old\ndescription: d\n---\n\nold body");
        let baseline = hash_tree(&library.join("old")).expect("baseline");
        let origin = Some(DraftOrigin { kind: "library".into(), label: "Your skills".into(), path: library.join("old/SKILL.md").display().to_string() });
        let (draft_root, manifest, review) = draft_with_skill("old", "new body", origin.clone(), Some(baseline.clone()));

        // Baseline moved on: refuse.
        write(&library.join("old/extra.txt"), "surprise");
        let error = do_publish(&library, &draft_root, &review.revision, &manifest, &review).unwrap_err();
        assert!(error.contains("changed on disk"));
        fs::remove_file(library.join("old/extra.txt")).expect("remove");

        // Baseline intact: swap succeeds, staging is cleaned, the old body is gone.
        let outcome = do_publish(&library, &draft_root, &review.revision, &manifest, &review).expect("swap");
        assert!(outcome.overwritten);
        let text = fs::read_to_string(library.join("old/SKILL.md")).expect("read");
        assert!(text.contains("new body"));
        let leftovers: Vec<_> = fs::read_dir(&library).expect("read").flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("."))
            .collect();
        assert!(leftovers.is_empty());

        // An interrupted swap whose target provably never changed is rolled back by status.
        let (interrupted_root, _interrupted_manifest, interrupted_review) = draft_with_skill("old", "third body", origin, Some(baseline.clone()));
        let journal = PublicationJournal {
            v: 1,
            state: "committing".into(),
            kind: "swap".into(),
            name: "old".into(),
            revision: interrupted_review.revision.clone(),
            target: library.join("old").display().to_string(),
            baseline: Some(baseline),
            staging: Some(library.join(".wackcode-skillpub-fake").display().to_string()),
            saved_at: None,
        };
        // Put the library back to the baseline the journal expects.
        write(&library.join("old/SKILL.md"), "---\nname: old\ndescription: d\n---\n\nold body");
        write_json_atomic(&journal_path(&interrupted_root), &journal).expect("journal");
        let reconciled = reconcile_journal(&interrupted_root, &journal).expect("reconcile");
        assert!(reconciled.is_none(), "a swap that never landed is not saved");
        assert!(!journal_path(&interrupted_root).exists());
    }

    #[test]
    fn publish_rewrites_a_loose_library_file_in_place_only_while_single_file() {
        let library = temp("library4");
        write(&library.join("loose.md"), "---\nname: loose\ndescription: d\n---\n\nold");
        let baseline = hash_single_file(&library.join("loose.md")).expect("baseline");
        let origin = Some(DraftOrigin { kind: "library".into(), label: "Your skills".into(), path: library.join("loose.md").display().to_string() });
        let (draft_root, manifest, review) = draft_with_skill("loose", "new instructions", origin.clone(), Some(baseline.clone()));
        let outcome = do_publish(&library, &draft_root, &review.revision, &manifest, &review).expect("file publish");
        assert!(outcome.overwritten);
        let text = fs::read_to_string(&outcome.path).expect("read");
        assert!(text.contains("new instructions"));

        // A loose-origin draft that grew supporting files cannot stay loose: it needs a free name.
        let (grown_root, grown_manifest, grown_review) = draft_with_skill("loose", "with files", origin, Some(baseline));
        write(&grown_root.join("skill/scripts/run.sh"), "#!/bin/sh\n");
        let revision = hash_tree(&grown_root.join("skill")).expect("hash");
        let grown_review = ReviewManifest { revision: revision.clone(), file_count: 2, ..grown_review.clone() };
        copy_tree_strict(&grown_root.join("skill"), &grown_root.join("review").join(&revision)).expect("copy");
        write_json_atomic(&grown_root.join("review").join(format!("{revision}.json")), &grown_review).expect("manifest");
        let error = do_publish(&library, &grown_root, &revision, &grown_manifest, &grown_review).unwrap_err();
        assert!(error.contains("already exists"), "the loose file itself blocks the folder name: {error}");
    }

    #[test]
    fn resolve_target_keeps_standard_folders_and_promotes_richer_loose_drafts() {
        let library = Path::new("/tmp/some-library");
        let external = Some(DraftOrigin { kind: "external".into(), label: "Claude Code".into(), path: "/x/SKILL.md".into() });
        assert_eq!(resolve_target(library, &None, "new-skill", true, "new-skill").unwrap(), Target::New);
        assert_eq!(resolve_target(library, &external, "copied", true, "copied").unwrap(), Target::New);
        let folder_origin = Some(DraftOrigin { kind: "library".into(), label: "Your skills".into(), path: library.join("foo/SKILL.md").display().to_string() });
        assert_eq!(resolve_target(library, &folder_origin, "foo", false, "foo").unwrap(), Target::SwapFolder(library.join("foo")));
        // A renamed folder update refuses to swap into a differently named folder.
        assert_eq!(resolve_target(library, &folder_origin, "bar", false, "foo").unwrap(), Target::New);
        let loose_origin = Some(DraftOrigin { kind: "library".into(), label: "Your skills".into(), path: library.join("loose.md").display().to_string() });
        assert_eq!(resolve_target(library, &loose_origin, "loose", true, "loose").unwrap(), Target::LooseFile(library.join("loose.md")));
        assert_eq!(resolve_target(library, &loose_origin, "loose", false, "loose").unwrap(), Target::New);
        assert_eq!(resolve_target(library, &loose_origin, "renamed", true, "loose").unwrap(), Target::New);
    }
}
