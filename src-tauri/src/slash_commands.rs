//! Slash commands (Settings › Commands): the user's own commands in `<app data>/commands`, and
//! the keyless scan that lists what a chat's `/` menu offers.
//!
//! A custom command is an ordinary Pi prompt template: `<name>.md`, optional frontmatter
//! `description` and `argument-hint`, then the body `/name args` expands (`$1`, `$ARGUMENTS`,
//! `${1:-default}` — see the worker's `slash.ts`). WackCode writes only inside the commands
//! folder; every path the renderer sends back is checked against it (canonically, so a symlink
//! can't lead out), and deleting moves a file to the Trash rather than erasing it.
//!
//! The scan (`commands-scan.js`) executes trusted extension code — the only way an extension
//! command's name exists — inside a throwaway process with no key, a provider-key-stripped
//! environment and PI_OFFLINE. Settings shows exactly what a chat loads because both go through
//! the same resolution code (`resolveCommandNames`).

use crate::models::{
    CommandsConfig, PackageRecord, SkillDiagnostic, SkillsConfig, SlashCommand, SlashCommandEntry,
    SlashCommandGroup, SlashCommandGroupKind, SlashCommandKind, SlashCommandsOverview,
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

const MAX_NAME_CHARS: usize = 64;
const MAX_DESCRIPTION_CHARS: usize = 1_024;
const MAX_HINT_CHARS: usize = 256;
const MAX_BODY_CHARS: usize = 200_000;
const MAX_DISABLED: usize = 2_000;
const MAX_PATH_CHARS: usize = 4_096;
const SCAN_TIMEOUT: Duration = Duration::from_secs(15);

/// The app's own commands, reserved by the picker before anything else loads.
pub const APP_COMMAND_NAMES: &[&str] = &["compact", "init", "new", "name", "copy", "goal"];

pub fn dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|_| "The app data folder could not be found.".to_string())?
        .join("commands"))
}

/// The `commands` value for `init` and `set_commands`: the folder the worker reads user
/// templates from plus the switched-off keys. Deliberately never part of the fingerprint, like
/// skills — toggling a command must not respawn a chat.
pub fn payload(config: &CommandsConfig, dir: &Path) -> Value {
    json!({ "dir": dir, "disabled": config.disabled })
}

// ---------------------------------------------------------------------------------------------
// Validation

pub fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the command a name.".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("A command name can be at most {MAX_NAME_CHARS} characters."));
    }
    if !name.chars().all(|character| character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-')
        || name.starts_with('-')
        || name.ends_with('-')
        || name.contains("--")
    {
        return Err("Use only lowercase letters, numbers and single hyphens, like review-diff.".into());
    }
    Ok(name.to_string())
}

pub fn validate_description(description: &str) -> Result<String, String> {
    let description = description.trim();
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(format!("A description can be at most {MAX_DESCRIPTION_CHARS} characters."));
    }
    Ok(description.to_string())
}

pub fn validate_hint(hint: &str) -> Result<String, String> {
    let hint = hint.trim();
    if hint.chars().count() > MAX_HINT_CHARS {
        return Err(format!("An argument hint can be at most {MAX_HINT_CHARS} characters."));
    }
    Ok(hint.to_string())
}

pub fn validate_body(body: &str) -> Result<String, String> {
    let body = body.trim();
    if body.is_empty() {
        return Err("The command needs instructions to expand to.".into());
    }
    if body.chars().count() > MAX_BODY_CHARS {
        return Err(format!("A command's instructions can be at most {MAX_BODY_CHARS} characters."));
    }
    Ok(body.to_string())
}

/// A path the renderer sends back: absolute, a `.md` file, and strictly inside the commands dir.
fn command_file(path: &str, dir: &Path) -> Result<PathBuf, String> {
    let path = Path::new(path);
    if path.as_os_str().len() > MAX_PATH_CHARS
        || !path.is_absolute()
        || path.components().any(|component| component == std::path::Component::ParentDir)
        || path.extension().and_then(|extension| extension.to_str()) != Some("md")
    {
        return Err("That is not a command file.".into());
    }
    crate::skills::inside(path, dir)
        .filter(|file| file.is_file())
        .ok_or_else(|| "Only commands in your commands folder can be changed here.".to_string())
}

/// A `key` from the Settings denylist: `app:` must be one of WackCode's own, `custom:` a file in
/// the commands dir, `prompt:`/`extension:`/`skill:` a resource file of a trusted package.
pub fn validate_key(key: &str, dir: &Path, packages: &[PackageRecord]) -> Result<(), String> {
    let (kind, rest) = key.split_once(':').ok_or("That command key is not recognised.")?;
    match kind {
        "app" => {
            if APP_COMMAND_NAMES.contains(&rest) { Ok(()) } else { Err("That is not one of WackCode's commands.".into()) }
        }
        "custom" => {
            command_file(rest, dir).map(|_| ())
        }
        "prompt" | "extension" | "skill" => {
            // Only extension keys append `#name`; prompt and skill keys are `kind:path` verbatim,
            // where the path itself may contain a `#`.
            let file = if kind == "extension" { rest.rsplit_once('#').map_or(rest, |pair| pair.0) } else { rest };
            let known = packages.iter()
                .filter(|package| !package.trusted_at.is_empty())
                .flat_map(|package| {
                    package.extensions.iter()
                        .chain(package.prompts.iter())
                        .chain(package.skills.iter())
                        .map(|resource| resource.path.as_str())
                })
                .any(|path| path == file || file.starts_with(&format!("{path}/")));
            if known { Ok(()) } else { Err("That command does not come from a trusted package.".into()) }
        }
        _ => Err("That command key is not recognised.".into()),
    }
}

/// Switch one command on or off. The denylist keeps the key exactly as the scan spelled it.
pub fn set_disabled(config: &mut CommandsConfig, key: &str, enabled: bool) -> Result<(), String> {
    config.disabled.retain(|entry| entry != key);
    if !enabled {
        if config.disabled.len() >= MAX_DISABLED {
            return Err("Too many commands are switched off. Switch some back on first.".into());
        }
        config.disabled.push(key.to_string());
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Template text

/// A new command file: filename is the name; frontmatter carries `description` and
/// `argument-hint`; the rest is the body `/name` expands.
pub fn compose(description: &str, hint: &str, body: &str) -> String {
    let mut out = String::from("---\n");
    if !description.is_empty() {
        out.push_str(&format!("description: {}\n", crate::skills::yaml_scalar(description)));
    }
    if !hint.is_empty() {
        out.push_str(&format!("argument-hint: {}\n", crate::skills::yaml_scalar(hint)));
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

// ---------------------------------------------------------------------------------------------
// Files

fn taken(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// A new command at `<dir>/<name>.md`.
pub fn create(dir: &Path, name: &str, description: &str, hint: &str, body: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|error| format!("Could not create the commands folder: {error}"))?;
    let file = dir.join(format!("{name}.md"));
    if taken(&file) {
        return Err(format!("A command named {name} already exists."));
    }
    crate::skills::write_atomic(&file, &compose(description, hint, body))?;
    Ok(file)
}

/// Rewrite a command in the dir. Renaming the command renames the file; returns the path
/// afterwards, spelled like `path` was.
pub fn update(dir: &Path, path: &str, name: &str, description: &str, hint: &str, body: &str) -> Result<PathBuf, String> {
    let file = command_file(path, dir)?;
    let requested = PathBuf::from(path);
    let target = file.with_file_name(format!("{name}.md"));
    if target != file && taken(&target) {
        return Err(format!("A command named {name} already exists."));
    }
    crate::skills::write_atomic(&file, &compose(description, hint, body))?;
    if target != file {
        fs::rename(&file, &target).map_err(|error| format!("The command was saved, but its file could not be renamed: {error}"))?;
    }
    Ok(if target != file { requested.with_file_name(format!("{name}.md")) } else { requested })
}

/// Move a command file to the Trash.
pub fn delete(dir: &Path, path: &str) -> Result<(), String> {
    crate::skills::move_to_trash(&command_file(path, dir)?, "command")
}

/// The template's body, for the editor.
pub fn read_document(dir: &Path, path: &str) -> Result<String, String> {
    let file = command_file(path, dir)?;
    let text = fs::read_to_string(&file).map_err(|error| format!("Could not read the command: {error}"))?;
    Ok(crate::skills::split(&text).1)
}

// ---------------------------------------------------------------------------------------------
// Scanning

#[derive(Debug, Deserialize)]
struct ScanLine {
    ok: bool,
    #[serde(default)]
    custom: Vec<ScannedCommand>,
    #[serde(default)]
    packages: Vec<ScannedGroup>,
    #[serde(default)]
    catalog: Vec<SlashCommand>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ScannedGroup {
    id: String,
    #[serde(default)]
    commands: Vec<ScannedCommand>,
    #[serde(default)]
    diagnostics: Vec<ScannedDiagnostic>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScannedCommand {
    key: String,
    name: String,
    raw_name: Option<String>,
    description: Option<String>,
    argument_hint: Option<String>,
    kind: String,
    file_path: Option<String>,
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct ScannedDiagnostic {
    #[serde(rename = "type")]
    kind: String,
    message: String,
    #[serde(default)]
    path: Option<String>,
}

/// Run Pi's resource loader over trusted packages' extensions, prompts and skills plus the user's
/// command and skill folders in a short-lived process (`commands-scan.js`). It executes trusted
/// extension code — how a command's name exists at all — with no key and no network.
async fn run_scan(app: &AppHandle, config: &CommandsConfig, skills_config: &SkillsConfig, packages: &[&PackageRecord], dir: &Path, cwd: &Path) -> Result<ScanLine, String> {
    let home = crate::skills::home_dir(app)?;
    let skill_folders = crate::skills::folders(skills_config, &home);
    let request = json!({
        "cwd": cwd,
        "packages": packages.iter().map(|package| json!({
            "source": package.source,
            "label": package.display_name,
            "installedPath": package.installed_path,
            "extensions": package.extensions.iter().map(|resource| json!({
                "path": resource.path, "enabled": resource.enabled,
            })).collect::<Vec<_>>(),
            "skills": package.skills.iter().map(|resource| json!({
                "path": resource.path, "enabled": resource.enabled,
            })).collect::<Vec<_>>(),
            "prompts": package.prompts.iter().map(|resource| json!({
                "path": resource.path, "enabled": resource.enabled,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "skillRoots": skill_folders.iter().filter(|folder| folder.enabled).map(|folder| json!({
            "path": folder.path, "label": folder.label,
        })).collect::<Vec<_>>(),
        "skillDisabled": skills_config.disabled,
        "commandsDir": dir,
        "disabled": config.disabled,
    });
    let agent_dir = app.path().app_data_dir().map_err(|error| error.to_string())?.join("agent");
    let mut command = Command::new(crate::worker::node_executable_path()?);
    command
        .arg(crate::worker::commands_scan_entry_path(app)?)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .env("PI_TELEMETRY", "0")
        .env("PI_SKIP_VERSION_CHECK", "1")
        .env("PI_OFFLINE", "1");
    crate::worker::strip_provider_env(&mut command);
    let mut child = command.spawn().map_err(|error| format!("Could not read your commands: {error}"))?;
    let mut stdin = child.stdin.take().ok_or("Could not read your commands.")?;
    let mut line = serde_json::to_vec(&json!({ "request": request, "agentDir": agent_dir })).map_err(|error| error.to_string())?;
    line.push(b'\n');
    stdin.write_all(&line).await.map_err(|error| format!("Could not read your commands: {error}"))?;
    drop(stdin);
    let output = tokio::time::timeout(SCAN_TIMEOUT, child.wait_with_output()).await
        .map_err(|_| "Reading your commands took too long.".to_string())?
        .map_err(|error| format!("Could not read your commands: {error}"))?;
    let result = String::from_utf8_lossy(&output.stdout)
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<ScanLine>(line).ok())
        .ok_or("Reading your commands stopped without an answer.")?;
    if !result.ok {
        return Err(format!(
            "Could not read your commands: {}",
            crate::worker::redact_and_limit(result.error.as_deref().unwrap_or("unknown error"))
        ));
    }
    Ok(result)
}

/// Settings' list: the user's own commands first, then every trusted package's commands.
pub async fn overview(app: &AppHandle) -> Result<SlashCommandsOverview, String> {
    let dir = dir(app)?;
    let home = crate::skills::home_dir(app)?;
    let (config, skills_config, packages) = {
        let state = app.state::<crate::storage::MetadataState>();
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        (data.commands.clone(), data.skills.clone(), data.packages.clone())
    };
    let trusted: Vec<&PackageRecord> = packages.iter()
        .filter(|package| !package.trusted_at.is_empty())
        .filter(|package| !package.extensions.is_empty() || !package.skills.is_empty() || !package.prompts.is_empty())
        .collect();
    let scanned = run_scan(app, &config, &skills_config, &trusted, &dir, &home).await?;
    let canonical_dir = dir.canonicalize().unwrap_or_else(|_| dir.clone());

    let kind_of = |kind: &str| match kind {
        "extension" => SlashCommandKind::Extension,
        "prompt" => SlashCommandKind::Prompt,
        _ => SlashCommandKind::Custom,
    };
    let entry_of = |command: &ScannedCommand, disabled: &CommandsConfig| -> SlashCommandEntry {
        let kind = kind_of(&command.kind);
        SlashCommandEntry {
            key: command.key.clone(),
            name: command.name.clone(),
            raw_name: command.raw_name.clone(),
            description: command.description.clone().unwrap_or_default(),
            argument_hint: command.argument_hint.clone(),
            kind,
            enabled: command.enabled && !disabled.disabled.contains(&command.key),
            editable: kind == SlashCommandKind::Custom
                && command.file_path.as_deref()
                    .and_then(|path| crate::skills::inside(Path::new(path), &canonical_dir))
                    .is_some(),
            file_path: command.file_path.clone(),
        }
    };
    let diagnostics_of = |group: &ScannedGroup| group.diagnostics.iter().map(|diagnostic| SkillDiagnostic {
        kind: diagnostic.kind.clone(),
        message: diagnostic.message.clone(),
        path: diagnostic.path.clone(),
    }).collect::<Vec<_>>();

    let mut groups = vec![SlashCommandGroup {
        id: "custom".into(),
        label: "Your commands".into(),
        kind: SlashCommandGroupKind::Custom,
        entries: scanned.custom.iter().map(|command| entry_of(command, &config)).collect(),
        diagnostics: Vec::new(),
    }];
    groups.extend(trusted.iter().filter_map(|package| {
        let group = scanned.packages.iter().find(|group| group.id == package.source)?;
        Some(SlashCommandGroup {
            id: package.source.clone(),
            label: package.display_name.clone(),
            kind: SlashCommandGroupKind::Package,
            entries: group.commands.iter().map(|command| entry_of(command, &config)).collect(),
            diagnostics: diagnostics_of(group),
        })
    }));
    Ok(SlashCommandsOverview {
        custom_path: dir.display().to_string(),
        disabled: config.disabled.clone(),
        groups,
    })
}

/// The full taskless `/` catalog for the welcome composer. It uses the same keyless scanner as
/// Settings, including skills, and never creates a chat or starts a provider-backed worker.
pub async fn catalog(app: &AppHandle, cwd: &Path) -> Result<Vec<SlashCommand>, String> {
    let dir = dir(app)?;
    let (config, skills_config, packages) = {
        let state = app.state::<crate::storage::MetadataState>();
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        (data.commands.clone(), data.skills.clone(), data.packages.clone())
    };
    let trusted: Vec<&PackageRecord> = packages.iter()
        .filter(|package| !package.trusted_at.is_empty())
        .filter(|package| !package.extensions.is_empty() || !package.skills.is_empty() || !package.prompts.is_empty())
        .collect();
    Ok(run_scan(app, &config, &skills_config, &trusted, &dir, cwd).await?.catalog)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("commands");
        (temp, path)
    }

    #[test]
    fn validates_names_and_composes_templates() {
        assert!(validate_name("review-diff").is_ok());
        assert!(validate_name("Review").is_err());
        assert!(validate_name("--lead").is_err());
        let text = compose("Look at it", "<files>", "Review $ARGUMENTS\nDone.");
        let (frontmatter, body) = crate::skills::split(&text);
        let frontmatter = frontmatter.unwrap();
        assert!(frontmatter.contains("description: \"Look at it\"") || frontmatter.contains("description: Look at it"));
        assert!(frontmatter.contains("argument-hint: \"<files>\"") || frontmatter.contains("argument-hint: <files>"));
        assert_eq!(body, "Review $ARGUMENTS\nDone.");
    }

    #[test]
    fn creates_updates_renames_and_reads() {
        let (_temp, dir) = dir();
        let file = create(&dir, "first", "One", "", "Body $1").unwrap();
        assert_eq!(read_document(&dir, file.to_str().unwrap()).unwrap(), "Body $1");
        let moved = update(&dir, file.to_str().unwrap(), "second", "Two", "", "New body").unwrap();
        assert!(moved.ends_with("second.md"));
        assert!(fs::read_to_string(&moved).unwrap().contains("New body"));
        assert!(!file.exists());
        assert!(update(&dir, "outside.md", "x", "", "", "y").is_err());
    }

    #[test]
    fn refuses_writes_outside_the_commands_dir() {
        let (_temp, dir) = dir();
        assert!(command_file("/etc/passwd.md", &dir).is_err());
        assert!(command_file("/tmp/../etc/x.md", &dir).is_err());
        assert!(read_document(&dir, "/etc/hosts.md").is_err());
    }

    #[test]
    fn validates_denylist_keys() {
        let (_temp, dir) = dir();
        assert!(validate_key("app:compact", &dir, &[]).is_ok());
        assert!(validate_key("app:goal", &dir, &[]).is_ok());
        assert!(validate_key("app:nope", &dir, &[]).is_err());
        assert!(validate_key("custom:/etc/x.md", &dir, &[]).is_err());
        let file = create(&dir, "mine", "", "", "b").unwrap();
        assert!(validate_key(&format!("custom:{}", file.display()), &dir, &[]).is_ok());
        assert!(validate_key("weird:/x", &dir, &[]).is_err());
    }

    #[test]
    fn denies_and_allows_commands() {
        let mut config = CommandsConfig::default();
        set_disabled(&mut config, "custom:/x", false).unwrap();
        assert_eq!(config.disabled, vec!["custom:/x".to_string()]);
        set_disabled(&mut config, "custom:/x", false).unwrap();
        assert_eq!(config.disabled.len(), 1);
        set_disabled(&mut config, "custom:/x", true).unwrap();
        assert!(config.disabled.is_empty());
    }
}
