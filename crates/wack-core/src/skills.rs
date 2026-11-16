//! Where skills and the user's commands come from, what is switched on, and what Settings ›
//! Skills and Settings › Commands list. The skill format, discovery and expansion are
//! `wack_agent::skills`' and `wack_agent::commands`', re-exported here.
//!
//! - **Skill folders,** in the order that settles a name clash: `~/.agents/skills` ("Your
//!   skills", always on and the only folder the app writes to), then other tools' folders the
//!   user switched on (Claude Code, Codex, OpenCode; off by default), then folders they added.
//!   A project's own skill folders are never read.
//! - **Commands** are Markdown files in `<data>/commands`, the file name being the command.
//! - **Switches** are command keys (`skill:<file>`, `template:<file>`, `builtin:<name>`) in
//!   `state.json`. A switched-off skill is gone from the prompt and the `/` menu alike.
//! - Nothing is cached: a run loads its skills as it starts, and the `/` menu reads the
//!   catalogue each time it opens, so an edit in another app shows at once. Both read only small
//!   files from a few folders.
//! - Deleting moves to the macOS Trash, through the function the caller passes (the UI's uses
//!   Finder's, so Put Back works); an import never overwrites a skill.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command as Process,
};

use serde::{Deserialize, Serialize};

pub use wack_agent::{
    commands::{
        Action, BUILTINS, Builtin, Command, CommandUse, Kind, Template, active_slash, builtin_key, compose_template, expand_template, file_key,
        find, matching, parse_line, pick, valid_name as valid_command_name,
    },
    skills::{Catalog, Document, Problem, Root, Shadowed, Skill, read_document},
};
use wack_agent::{commands, skills};

/// A skill file's instructions, without its frontmatter.
pub fn split_body(text: &str) -> String {
    skills::frontmatter::split(text).1
}

pub use wack_agent::skills::{validate_description, validate_hint, validate_name};

use crate::store::Store;

const MAX_ADDED_FOLDERS: usize = 16;
const MAX_OFF: usize = 2_000;
const MAX_COPY_BYTES: u64 = 50 * 1024 * 1024;
const MAX_COPY_FILES: usize = 2_000;

pub const LIBRARY: &str = "library";
const LIBRARY_LABEL: &str = "Your skills";
const ADDED_PREFIX: &str = "added:";

/// Other tools' folders, which the user may switch on.
const TOOL_FOLDERS: &[(&str, &str, &str)] = &[
    ("claude", "Claude Code", ".claude/skills"),
    ("codex", "Codex", ".codex/skills"),
    ("opencode", "OpenCode", ".config/opencode/skills"),
];

/// Settings › Skills' and Settings › Commands' switches.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillsConfig {
    /// Ids of other tools' folders that are switched on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_folders: Vec<String>,
    /// Folders the user added, in the order they were added.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added_folders: Vec<PathBuf>,
    /// Keys of switched-off skills and commands.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub off: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderKind {
    Library,
    Tool,
    Added,
}

/// One skill folder as Settings shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct SkillFolder {
    /// `library`, a tool's id, or `added:<path>`.
    pub id: String,
    pub label: String,
    pub path: PathBuf,
    /// The path with `~` for the home folder.
    pub display: String,
    pub kind: FolderKind,
    pub enabled: bool,
    pub skills: Vec<SkillEntry>,
    pub problems: Vec<Problem>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkillEntry {
    pub skill: Skill,
    /// Its own switch, whatever its folder's.
    pub enabled: bool,
    /// The folder of the skill that took its name, when it lost one.
    pub shadowed_by: Option<String>,
}

/// One command as Settings › Commands shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandEntry {
    pub command: Command,
    pub enabled: bool,
}

/// What sending a message does.
#[derive(Clone, Debug, PartialEq)]
pub enum Outgoing {
    /// An ordinary message.
    Message(String),
    /// A command the model gets: `text` is what it is sent, `command` what the user typed.
    Prompt { text: String, command: CommandUse },
    /// One of WackCode's commands that acts on the app.
    Builtin { name: &'static str, args: String },
}

/// What `text` sends, against the catalogue `commands`. `Err` is a sentence for the user: an
/// unknown command, or a built-in given the wrong arguments.
pub fn resolve_message(commands: &[Command], text: &str, cwd: &Path) -> Result<Outgoing, String> {
    let Some((name, args)) = parse_line(text) else { return Ok(Outgoing::Message(text.to_string())) };
    let Some(command) = find(commands, name) else {
        return Err(format!("There is no /{name} command. To send the text as it is, choose Send as message."));
    };
    if let Action::Builtin(builtin) = &command.action {
        match builtin.name {
            "new" | "copy" | "init" if !args.is_empty() => return Err(format!("/{} takes nothing after it.", builtin.name)),
            "name" if args.is_empty() => return Err("Type the chat's new name after /name.".into()),
            _ => {}
        }
    }
    let used = CommandUse { name: command.name.clone(), args: args.to_string(), kind: command.kind };
    match commands::expand(command, args, cwd) {
        Some(Ok(text)) => Ok(Outgoing::Prompt { text, command: used }),
        Some(Err(message)) => Err(message),
        None => {
            let Action::Builtin(builtin) = &command.action else { unreachable!("only built-ins act on the app") };
            Ok(Outgoing::Builtin { name: builtin.name, args: args.to_string() })
        }
    }
}

/// `path` with `~` for `home`.
fn display(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

fn added_label(path: &Path) -> String {
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
    // `…/tool/skills` reads better as `tool/skills` than as another "skills".
    match path.parent().and_then(Path::file_name) {
        Some(parent) if name == "skills" => format!("{}/skills", parent.to_string_lossy()),
        _ => name,
    }
}

impl Store {
    fn home(&self) -> PathBuf {
        self.home_override().cloned().unwrap_or_else(|| std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/")))
    }

    /// `~/.agents/skills`, the only skill folder the app writes to.
    pub fn skills_library(&self) -> PathBuf {
        self.home().join(".agents/skills")
    }

    /// `<data>/commands`, where the user's commands live.
    pub fn commands_folder(&self) -> PathBuf {
        self.folder().join("commands")
    }

    /// Every skill folder in load order, switched on or not.
    pub fn skill_folders(&self) -> Vec<(String, String, PathBuf, FolderKind, bool)> {
        let home = self.home();
        let config = self.skills_config();
        let mut folders = vec![(LIBRARY.to_string(), LIBRARY_LABEL.to_string(), self.skills_library(), FolderKind::Library, true)];
        for (id, label, path) in TOOL_FOLDERS {
            folders.push((id.to_string(), label.to_string(), home.join(path), FolderKind::Tool, config.tool_folders.iter().any(|on| on == id)));
        }
        for path in &config.added_folders {
            folders.push((format!("{ADDED_PREFIX}{}", path.display()), added_label(path), path.clone(), FolderKind::Added, true));
        }
        folders
    }

    /// The folders a run loads skills from, in order.
    pub fn skill_roots(&self) -> Vec<Root> {
        self.skill_folders().into_iter().filter(|folder| folder.4).map(|(_, label, path, _, _)| Root { path, label }).collect()
    }

    /// Skill files switched off one by one.
    pub fn disabled_skills(&self) -> HashSet<PathBuf> {
        self.skills_config().off.iter().filter_map(|key| key.strip_prefix("skill:")).map(PathBuf::from).collect()
    }

    /// The skills a run loads now. Reads the disk.
    pub fn skill_catalog(&self) -> Catalog {
        skills::load(&self.skill_roots(), &self.disabled_skills())
    }

    /// Every command the `/` menu offers now, names resolved. Reads the disk.
    pub fn command_catalog(&self) -> Vec<Command> {
        let off: HashSet<String> = self.skills_config().off.iter().cloned().collect();
        commands::catalog(&commands::load_templates(&self.commands_folder()), &self.skill_catalog().skills, &off)
    }

    /// Settings › Skills: every folder, switched on or not, each skill marked with whether it is
    /// on and what took its name. Reads the disk.
    pub fn skills_overview(&self) -> Vec<SkillFolder> {
        let home = self.home();
        let disabled = self.disabled_skills();
        let loaded = self.skill_catalog();
        let shadowed: Vec<(&Path, &str)> = loaded.shadowed.iter().map(|lost| (lost.skill.file.as_path(), lost.by.as_str())).collect();
        self.skill_folders()
            .into_iter()
            .map(|(id, label, path, kind, enabled)| {
                let (found, problems) = skills::scan_root(&Root { path: path.clone(), label: label.clone() });
                let skills = found
                    .into_iter()
                    .map(|skill| SkillEntry {
                        enabled: !disabled.contains(&skill.file),
                        shadowed_by: shadowed.iter().find(|(file, _)| *file == skill.file).map(|(_, by)| by.to_string()),
                        skill,
                    })
                    .collect();
                SkillFolder { display: display(&path, &home), id, label, path, kind, enabled, skills, problems }
            })
            .collect()
    }

    /// Settings › Commands: WackCode's commands and the user's, each with its switch.
    pub fn commands_overview(&self) -> Vec<CommandEntry> {
        let off: HashSet<String> = self.skills_config().off.iter().cloned().collect();
        commands::catalog(&commands::load_templates(&self.commands_folder()), &[], &HashSet::new())
            .into_iter()
            .map(|command| CommandEntry { enabled: !off.contains(&command.key), command })
            .collect()
    }

    /// Switches one skill or command on or off by its key.
    pub fn set_command_enabled(&mut self, key: &str, enabled: bool) -> Result<(), String> {
        let known = key.strip_prefix("builtin:").is_some_and(|name| BUILTINS.iter().any(|builtin| builtin.name == name))
            || key.strip_prefix("skill:").or_else(|| key.strip_prefix("template:")).is_some_and(|path| Path::new(path).is_absolute());
        if !known {
            return Err("That skill or command is not recognised.".into());
        }
        let config = self.skills_config_mut();
        let before = config.off.clone();
        config.off.retain(|entry| entry != key);
        if !enabled {
            if config.off.len() >= MAX_OFF {
                return Err("Too many skills and commands are switched off. Switch some back on first.".into());
            }
            config.off.push(key.to_string());
        }
        if config.off == before {
            return Ok(());
        }
        self.save_state()
    }

    /// Switches another tool's folder on or off. "Your skills" is always on; an added folder is
    /// removed instead.
    pub fn set_skill_folder_enabled(&mut self, id: &str, enabled: bool) -> Result<(), String> {
        if !TOOL_FOLDERS.iter().any(|(known, _, _)| *known == id) {
            return Err("Only other tools' folders can be switched off.".into());
        }
        let config = self.skills_config_mut();
        let before = config.tool_folders.clone();
        config.tool_folders.retain(|entry| entry != id);
        if enabled {
            config.tool_folders.push(id.to_string());
        }
        if config.tool_folders == before {
            return Ok(());
        }
        self.save_state()
    }

    /// Adds a folder to load skills from, after every other.
    pub fn add_skill_folder(&mut self, path: &Path) -> Result<(), String> {
        let path = path.canonicalize().map_err(|_| "That folder could not be found.".to_string())?;
        if !path.is_dir() {
            return Err("Choose a folder, not a file.".into());
        }
        if self.skill_folders().iter().any(|folder| folder.2 == path || folder.2.canonicalize().is_ok_and(|real| real == path)) {
            return Err("Skills already load from that folder.".into());
        }
        let config = self.skills_config_mut();
        if config.added_folders.len() >= MAX_ADDED_FOLDERS {
            return Err(format!("WackCode can load skills from at most {MAX_ADDED_FOLDERS} added folders."));
        }
        config.added_folders.push(path);
        self.save_state()
    }

    pub fn remove_skill_folder(&mut self, id: &str) -> Result<(), String> {
        let path = id.strip_prefix(ADDED_PREFIX).ok_or("Only folders you added can be removed.")?;
        let config = self.skills_config_mut();
        let before = config.added_folders.len();
        config.added_folders.retain(|folder| folder != Path::new(path));
        if config.added_folders.len() == before {
            return Ok(());
        }
        self.save_state()
    }

    /// A skill file inside "Your skills", for the editor to change. Anything else is refused.
    /// The path comes back as given, so it still matches the switch keys the scan made.
    fn library_file(&self, file: &Path) -> Result<PathBuf, String> {
        let library = self.skills_library().canonicalize().map_err(|_| "Only skills in Your skills can be changed here.".to_string())?;
        let real = file.canonicalize().map_err(|_| "That skill could not be found. It may have been moved or deleted.".to_string())?;
        if !real.starts_with(&library) || real.extension().is_none_or(|extension| extension != "md") {
            return Err("Only skills in Your skills can be changed here.".into());
        }
        Ok(file.to_path_buf())
    }

    /// Saves a new skill in "Your skills" as `<name>/SKILL.md`.
    pub fn create_skill(&self, name: &str, description: &str, manual: bool, hint: &str, body: &str) -> Result<PathBuf, String> {
        let (name, description, hint, body) = validate(name, description, hint, body)?;
        let folder = self.skills_library().join(&name);
        if fs::symlink_metadata(&folder).is_ok() {
            return Err(format!("Your skills already has one named {name}."));
        }
        fs::create_dir_all(&folder).map_err(|error| format!("Could not create the skill's folder: {error}"))?;
        let file = folder.join("SKILL.md");
        write_atomic(&file, &skills::compose(None, &name, &description, manual, &hint, &body))?;
        Ok(file)
    }

    /// Saves changes to a skill in "Your skills". A new name renames its folder too, when the
    /// folder is named after the skill. Returns where the file now is.
    pub fn update_skill(&mut self, file: &Path, name: &str, description: &str, manual: bool, hint: &str, body: &str) -> Result<PathBuf, String> {
        let file = self.library_file(file)?;
        let (name, description, hint, body) = validate(name, description, hint, body)?;
        let original = fs::read_to_string(&file).map_err(|error| format!("Could not read the skill: {error}"))?;
        write_atomic(&file, &skills::compose(Some(&original), &name, &description, manual, &hint, &body))?;
        let library = self.skills_library();
        let Some(folder) = file.parent().filter(|folder| file.file_name().is_some_and(|file| file == "SKILL.md") && folder.parent() == Some(library.as_path())) else {
            return Ok(file);
        };
        if folder.file_name().is_some_and(|current| current.to_string_lossy() == name) {
            return Ok(file);
        }
        let renamed = library.join(&name);
        if fs::symlink_metadata(&renamed).is_ok() {
            return Err(format!("The skill was saved, but its folder could not be renamed: Your skills already has a folder named {name}."));
        }
        fs::rename(folder, &renamed).map_err(|error| format!("The skill was saved, but its folder could not be renamed: {error}"))?;
        let moved = renamed.join("SKILL.md");
        self.move_switch(&file_key(Kind::Skill, &file), &file_key(Kind::Skill, &moved))?;
        Ok(moved)
    }

    /// A switched-off skill or command stays off when its file moves.
    fn move_switch(&mut self, from: &str, to: &str) -> Result<(), String> {
        let config = self.skills_config_mut();
        let Some(entry) = config.off.iter_mut().find(|entry| *entry == from) else { return Ok(()) };
        *entry = to.to_string();
        self.save_state()
    }

    /// Moves a skill in "Your skills" to the Trash with `trash`: its whole folder for a
    /// `SKILL.md`, the file for a loose one.
    pub fn delete_skill(&mut self, file: &Path, trash: &dyn Fn(&Path) -> Result<(), String>) -> Result<(), String> {
        let file = self.library_file(file)?;
        let library = self.skills_library();
        let target = match file.parent() {
            Some(folder) if file.file_name().is_some_and(|name| name == "SKILL.md") && folder != library => folder.to_path_buf(),
            _ => file.clone(),
        };
        trash(&target)?;
        let key = file_key(Kind::Skill, &file);
        if self.skills_config().off.contains(&key) {
            self.skills_config_mut().off.retain(|entry| *entry != key);
            self.save_state()?;
        }
        Ok(())
    }

    /// Copies a skill from another folder into "Your skills", where it can be edited. Its whole
    /// folder comes along for a `SKILL.md`.
    pub fn copy_skill_to_library(&self, skill: &Skill) -> Result<PathBuf, String> {
        let name = skills::validate_name(&skill.name).map_err(|_| format!("{} can't be copied: its name doesn't follow the skill format.", skill.name))?;
        let target = self.skills_library().join(&name);
        if fs::symlink_metadata(&target).is_ok() {
            return Err(format!("Your skills already has one named {name}."));
        }
        copy_skill(skill, &target)?;
        Ok(target.join("SKILL.md"))
    }

    /// Imports every skill in a folder, a single `.md` file, or a `.zip` into "Your skills".
    /// Skills whose names are taken are skipped and named in the result, never overwritten.
    /// Returns the names imported and the names skipped.
    pub fn import_skills(&self, source: &Path) -> Result<(Vec<String>, Vec<String>), String> {
        let is_zip = source.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("zip"));
        let staging = Staging::new(&self.folder().join(format!(".import-{}", std::process::id())))?;
        let root = if is_zip {
            unzip(source, &staging.0)?;
            staging.0.clone()
        } else {
            source.to_path_buf()
        };
        let (found, problems) = skills::scan_root(&Root { path: root, label: "Import".into() });
        if found.is_empty() {
            return Err(match problems.first() {
                Some(problem) => format!("No skill could be imported: {}", problem.message),
                None => "No skills were found there. A skill is a folder with a SKILL.md, or a .md file with a description.".into(),
            });
        }
        let (mut imported, mut skipped) = (Vec::new(), Vec::new());
        for skill in found {
            match self.copy_skill_to_library(&skill) {
                Ok(_) => imported.push(skill.name),
                Err(_) => skipped.push(skill.name),
            }
        }
        Ok((imported, skipped))
    }

    /// Saves one of the user's commands. `previous` is its name before this edit, so a rename
    /// moves the file (and its switch).
    pub fn save_template(&mut self, previous: Option<&str>, name: &str, description: &str, hint: &str, body: &str) -> Result<PathBuf, String> {
        let name = name.trim().trim_start_matches('/');
        if !valid_command_name(name) {
            return Err("Use lowercase letters, numbers and single hyphens in a command's name.".into());
        }
        if BUILTINS.iter().any(|builtin| builtin.name == name) {
            return Err(format!("/{name} is one of WackCode's own commands. Choose another name."));
        }
        if body.trim().is_empty() {
            return Err("Write the prompt the command sends.".into());
        }
        if body.chars().count() > commands::MAX_BODY_CHARS {
            return Err(format!("A command's prompt can be at most {} characters.", commands::MAX_BODY_CHARS));
        }
        skills::validate_hint(hint)?;
        let folder = self.commands_folder();
        fs::create_dir_all(&folder).map_err(|error| format!("Could not create the commands folder: {error}"))?;
        let file = folder.join(format!("{name}.md"));
        let old = previous.filter(|previous| valid_command_name(previous)).map(|previous| folder.join(format!("{previous}.md")));
        let renaming = old.as_ref().is_some_and(|old| *old != file);
        if (old.is_none() || renaming) && fs::symlink_metadata(&file).is_ok() {
            return Err(format!("You already have a command named /{name}."));
        }
        let original = old.as_ref().and_then(|old| fs::read_to_string(old).ok());
        write_atomic(&file, &compose_template(original.as_deref(), description, hint, body))?;
        if let Some(old) = old.filter(|_| renaming) {
            let _ = fs::remove_file(&old);
            self.move_switch(&file_key(Kind::Template, &old), &file_key(Kind::Template, &file))?;
        }
        Ok(file)
    }

    /// Moves one of the user's commands to the Trash with `trash`.
    pub fn delete_template(&mut self, name: &str, trash: &dyn Fn(&Path) -> Result<(), String>) -> Result<(), String> {
        if !valid_command_name(name) {
            return Err("That command is not recognised.".into());
        }
        let file = self.commands_folder().join(format!("{name}.md"));
        if fs::symlink_metadata(&file).is_err() {
            return Err("That command could not be found. It may have been deleted already.".into());
        }
        trash(&file)?;
        let key = file_key(Kind::Template, &file);
        if self.skills_config().off.contains(&key) {
            self.skills_config_mut().off.retain(|entry| *entry != key);
            self.save_state()?;
        }
        Ok(())
    }
}

/// A scratch folder that is removed when dropped.
struct Staging(PathBuf);

impl Staging {
    fn new(path: &Path) -> Result<Self, String> {
        let _ = fs::remove_dir_all(path);
        fs::create_dir_all(path).map_err(|error| format!("Could not prepare the import: {error}"))?;
        Ok(Self(path.to_path_buf()))
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn validate(name: &str, description: &str, hint: &str, body: &str) -> Result<(String, String, String, String), String> {
    Ok((skills::validate_name(name)?, skills::validate_description(description)?, skills::validate_hint(hint)?, skills::validate_body(body)?))
}

fn write_atomic(file: &Path, text: &str) -> Result<(), String> {
    let name = file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    // A leading dot keeps the half-written file out of every scan.
    let staging = file.with_file_name(format!(".{name}.saving"));
    fs::write(&staging, text).map_err(|error| format!("Could not save the file: {error}"))?;
    fs::rename(&staging, file).map_err(|error| {
        let _ = fs::remove_file(&staging);
        format!("Could not save the file: {error}")
    })
}

/// A skill's files into a new folder `target`: its whole folder for a `SKILL.md`, the file alone
/// (as `SKILL.md`) for a loose one. Symlinks are not followed or copied.
fn copy_skill(skill: &Skill, target: &Path) -> Result<(), String> {
    let staging = target.with_file_name(format!(".{}.copying", target.file_name().unwrap_or_default().to_string_lossy()));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(|error| format!("Could not copy the skill: {error}"))?;
    let result = if skill.file.file_name().is_some_and(|name| name == "SKILL.md") {
        let mut budget = (MAX_COPY_FILES, MAX_COPY_BYTES);
        copy_tree(&skill.base_dir, &staging, &mut budget)
    } else {
        fs::copy(&skill.file, staging.join("SKILL.md")).map(|_| ()).map_err(|error| format!("Could not copy the skill: {error}"))
    };
    if let Err(message) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(message);
    }
    fs::rename(&staging, target).map_err(|error| {
        let _ = fs::remove_dir_all(&staging);
        format!("Could not copy the skill: {error}")
    })
}

fn copy_tree(from: &Path, to: &Path, budget: &mut (usize, u64)) -> Result<(), String> {
    let entries = fs::read_dir(from).map_err(|error| format!("Could not read the skill's folder: {error}"))?;
    for entry in entries.filter_map(|entry| entry.ok()) {
        let name = entry.file_name();
        if name == ".git" || name == "node_modules" {
            continue;
        }
        let Ok(kind) = entry.file_type() else { continue };
        let target = to.join(&name);
        if kind.is_dir() {
            fs::create_dir_all(&target).map_err(|error| format!("Could not copy the skill: {error}"))?;
            copy_tree(&entry.path(), &target, budget)?;
        } else if kind.is_file() {
            let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
            if budget.0 == 0 || budget.1 < size {
                return Err("The skill's folder is too large to copy (over 2,000 files or 50 MB).".into());
            }
            budget.0 -= 1;
            budget.1 -= size;
            fs::copy(entry.path(), &target).map_err(|error| format!("Could not copy the skill: {error}"))?;
        }
    }
    Ok(())
}

/// Extracts a `.zip` into `into`, refusing an archive with an entry that would land outside it
/// (an absolute path or `..`), and removing any symlink it made.
fn unzip(archive: &Path, into: &Path) -> Result<(), String> {
    let listed = Process::new("/usr/bin/zipinfo").arg("-1").arg(archive).output().map_err(|error| format!("Could not read the archive: {error}"))?;
    if !listed.status.success() {
        return Err("That file is not a ZIP archive WackCode can read.".into());
    }
    let names = String::from_utf8_lossy(&listed.stdout);
    if names.lines().count() > MAX_COPY_FILES {
        return Err("The archive has too many files to import.".into());
    }
    for name in names.lines() {
        let path = Path::new(name);
        if path.is_absolute() || path.components().any(|part| matches!(part, std::path::Component::ParentDir)) {
            return Err("The archive has files that would land outside its folder, so it was not imported.".into());
        }
    }
    let status = Process::new("/usr/bin/ditto").args(["-x", "-k"]).arg(archive).arg(into).status().map_err(|error| format!("Could not open the archive: {error}"))?;
    if !status.success() {
        return Err("The archive could not be opened.".into());
    }
    remove_symlinks(into);
    Ok(())
}

fn remove_symlinks(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.filter_map(|entry| entry.ok()) {
        match entry.file_type() {
            Ok(kind) if kind.is_symlink() => {
                let _ = fs::remove_file(entry.path());
            }
            Ok(kind) if kind.is_dir() => remove_symlinks(&entry.path()),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn skill_text(name: &str, description: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\nDo the {name} thing.\n")
    }

    struct Fixture {
        _data: tempfile::TempDir,
        home: tempfile::TempDir,
        store: Store,
    }

    /// The tests' Trash: a folder in the fake home.
    fn trash(home: &Path) -> impl Fn(&Path) -> Result<(), String> + '_ {
        move |path: &Path| {
            let bin = home.join(".Trash");
            fs::create_dir_all(&bin).unwrap();
            fs::rename(path, bin.join(path.file_name().unwrap())).map_err(|error| error.to_string())
        }
    }

    fn fixture() -> Fixture {
        let data = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let mut store = Store::open(data.path()).unwrap();
        store.set_home_override(home.path().to_path_buf());
        Fixture { _data: data, home, store }
    }

    #[test]
    fn other_tools_folders_load_only_when_switched_on() {
        let mut f = fixture();
        write(&f.home.path().join(".agents/skills/pdf/SKILL.md"), &skill_text("pdf", "Mine"));
        write(&f.home.path().join(".claude/skills/pdf/SKILL.md"), &skill_text("pdf", "Claude's"));
        write(&f.home.path().join(".claude/skills/xlsx/SKILL.md"), &skill_text("xlsx", "Sheets"));
        assert_eq!(f.store.skill_catalog().skills.len(), 1);

        f.store.set_skill_folder_enabled("claude", true).unwrap();
        let catalog = f.store.skill_catalog();
        assert_eq!(catalog.skills.iter().map(|skill| skill.name.as_str()).collect::<Vec<_>>(), ["pdf", "xlsx"]);
        assert_eq!(catalog.skills[0].description, "Mine");

        let overview = f.store.skills_overview();
        let claude = overview.iter().find(|folder| folder.id == "claude").unwrap();
        assert_eq!(claude.display, "~/.claude/skills");
        assert_eq!(claude.skills[0].shadowed_by.as_deref(), Some("Your skills"));
        assert!(f.store.set_skill_folder_enabled(LIBRARY, false).is_err());
    }

    #[test]
    fn switches_survive_reopening_and_feed_the_catalogue() {
        let mut f = fixture();
        let file = f.home.path().join(".agents/skills/pdf/SKILL.md");
        write(&file, &skill_text("pdf", "Mine"));
        f.store.set_command_enabled(&file_key(Kind::Skill, &file), false).unwrap();
        f.store.set_command_enabled(&builtin_key("copy"), false).unwrap();
        assert!(f.store.set_command_enabled("nonsense", false).is_err());
        let mut reopened = Store::open(f.store.folder()).unwrap();
        reopened.set_home_override(f.home.path().to_path_buf());
        let names: Vec<String> = reopened.command_catalog().into_iter().map(|command| command.name).collect();
        assert_eq!(names, ["compact", "init", "new", "name"]);
        assert!(!reopened.skills_overview()[0].skills[0].enabled);
    }

    #[test]
    fn messages_resolve_against_the_catalogue() {
        let f = fixture();
        write(&f.home.path().join(".agents/skills/pdf/SKILL.md"), &skill_text("pdf", "Mine"));
        let mut store = f.store;
        store.save_template(None, "review", "Review a file", "<file>", "Review $1.").unwrap();
        let commands = store.command_catalog();
        let cwd = f.home.path();
        assert_eq!(resolve_message(&commands, "hello /pdf", cwd).unwrap(), Outgoing::Message("hello /pdf".into()));
        assert_eq!(resolve_message(&commands, "/tmp/x", cwd).unwrap(), Outgoing::Message("/tmp/x".into()));
        assert_eq!(
            resolve_message(&commands, "/review main.rs", cwd).unwrap(),
            Outgoing::Prompt { text: "Review main.rs.".into(), command: CommandUse { name: "review".into(), args: "main.rs".into(), kind: Kind::Template } }
        );
        let Outgoing::Prompt { text, command } = resolve_message(&commands, "/skill:pdf now", cwd).unwrap() else { panic!() };
        assert!(text.starts_with("<skill name=\"pdf\""));
        assert_eq!(command.name, "pdf");
        assert_eq!(resolve_message(&commands, "/name  Better ", cwd).unwrap(), Outgoing::Builtin { name: "name", args: "Better".into() });
        assert!(matches!(resolve_message(&commands, "/init", cwd).unwrap(), Outgoing::Prompt { command: CommandUse { kind: Kind::Builtin, .. }, .. }));
        assert!(resolve_message(&commands, "/new now", cwd).is_err());
        assert!(resolve_message(&commands, "/name", cwd).is_err());
        assert!(resolve_message(&commands, "/nope", cwd).unwrap_err().contains("/nope"));
    }

    #[test]
    fn the_editor_creates_renames_and_trashes() {
        let mut f = fixture();
        let file = f.store.create_skill("pdf", "Fill forms", false, "", "Body").unwrap();
        assert!(f.store.create_skill("pdf", "Again", false, "", "Body").is_err());
        assert!(f.store.create_skill("Bad Name", "x", false, "", "Body").is_err());
        let moved = f.store.update_skill(&file, "pdf-forms", "Fill forms", true, "[file]", "New body").unwrap();
        assert!(moved.ends_with(".agents/skills/pdf-forms/SKILL.md"));
        assert!(!file.exists());
        let document = read_document(&moved).unwrap();
        assert!(document.manual);
        assert_eq!(document.body, "New body");

        // Only "Your skills" can be edited.
        let other = f.home.path().join(".claude/skills/x/SKILL.md");
        write(&other, &skill_text("x", "Theirs"));
        assert!(f.store.update_skill(&other, "x", "y", false, "", "").is_err());
        assert!(f.store.delete_skill(&other, &trash(f.home.path())).is_err());

        f.store.delete_skill(&moved, &trash(f.home.path())).unwrap();
        assert!(!moved.parent().unwrap().exists());
        assert!(f.home.path().join(".Trash/pdf-forms/SKILL.md").exists());
    }

    #[test]
    fn importing_copies_and_never_overwrites() {
        let f = fixture();
        let source = tempfile::tempdir().unwrap();
        write(&source.path().join("pdf/SKILL.md"), &skill_text("pdf", "Theirs"));
        write(&source.path().join("pdf/scripts/fill.py"), "print(1)");
        write(&source.path().join("loose.md"), "---\ndescription: Loose\n---\nBody");
        let (imported, skipped) = f.store.import_skills(source.path()).unwrap();
        assert_eq!((imported, skipped), (vec!["loose".to_string(), "pdf".to_string()], Vec::<String>::new()));
        let library = f.home.path().join(".agents/skills");
        assert!(library.join("pdf/scripts/fill.py").exists());
        assert!(library.join("loose/SKILL.md").exists());
        let (imported, skipped) = f.store.import_skills(source.path()).unwrap();
        assert!(imported.is_empty());
        assert_eq!(skipped.len(), 2);
        assert!(f.store.import_skills(tempfile::tempdir().unwrap().path()).is_err());
    }

    #[test]
    fn importing_a_zip() {
        let f = fixture();
        let source = tempfile::tempdir().unwrap();
        write(&source.path().join("pdf/SKILL.md"), &skill_text("pdf", "Zipped"));
        let archive = source.path().join("pdf.zip");
        let status = Process::new("/usr/bin/ditto").args(["-c", "-k", "--keepParent"]).arg(source.path().join("pdf")).arg(&archive).status().unwrap();
        assert!(status.success());
        let (imported, _) = f.store.import_skills(&archive).unwrap();
        assert_eq!(imported, ["pdf"]);
        assert!(f.home.path().join(".agents/skills/pdf/SKILL.md").exists());
    }

    #[test]
    fn commands_save_rename_and_keep_their_switch() {
        let mut f = fixture();
        let file = f.store.save_template(None, "review", "", "", "Review $1.").unwrap();
        assert!(f.store.save_template(None, "review", "", "", "Again").is_err());
        assert!(f.store.save_template(None, "new", "", "", "Body").is_err());
        assert!(f.store.save_template(None, "Bad", "", "", "Body").is_err());
        assert!(f.store.save_template(None, "empty", "", "", " ").is_err());
        f.store.set_command_enabled(&file_key(Kind::Template, &file), false).unwrap();
        let renamed = f.store.save_template(Some("review"), "check", "Check a file", "", "Check $1.").unwrap();
        assert!(!file.exists());
        let overview = f.store.commands_overview();
        let check = overview.iter().find(|entry| entry.command.name == "check").unwrap();
        assert!(!check.enabled);
        assert_eq!(check.command.description, "Check a file");
        f.store.delete_template("check", &trash(f.home.path())).unwrap();
        assert!(!renamed.exists());
        assert!(f.store.skills_config().off.is_empty());
    }
}
