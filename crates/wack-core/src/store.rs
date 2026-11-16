//! The app's own data: where it lives, the provider connections, projects and chats.
//!
//! `state.json` describes a connection (name, address, models); its key is in `secrets.json`
//! under the connection's id. A key is read to build the provider that sends requests, and, only
//! when the user asks to see it in Settings › Providers, by `Store::reveal_connection_key`.
//!
//! A chat's conversation is its session file (`sessions/<file>`); `state.json` holds only what
//! the sidebar needs to list it. Every change is saved at once, written whole and renamed into
//! place, so a crash leaves the old file or the new one, never half of either.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use wack_agent::AgentMode;
use wack_llm::{Effort, HttpProvider, WireFormat};

use crate::{
    appearance::{Appearance, Backdrop},
    secrets::SecretStore,
    usage::{RunRecorder, UsageWriter, root_for},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    /// The model's context size in tokens, if the user gave one. It decides when chats compact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    /// The reasoning efforts the model's endpoint accepts (Settings › Providers, as version 2's
    /// "Supported efforts"). None listed means it does not reason, so it only has Off: models
    /// saved before the list existed load that way until the user picks theirs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub efforts: Vec<Reasoning>,
}

impl Model {
    /// A model that does not reason, as a new model row starts.
    pub fn new(id: impl Into<String>, context_window: Option<u64>) -> Self {
        Self { id: id.into(), context_window, efforts: Vec::new() }
    }

    /// The efforts the model accepts, low to high, never Ultra. Without a list, only Off.
    pub fn levels(&self) -> Vec<Reasoning> {
        let mut levels: Vec<Reasoning> = self.efforts.iter().copied().filter(|level| !level.is_ultra()).collect();
        levels.sort();
        levels.dedup();
        if levels.is_empty() {
            levels.push(Reasoning::Off);
        }
        levels
    }

    /// It can reason at all: it lists an effort above Off.
    pub fn reasons(&self) -> bool {
        self.levels().iter().any(|level| *level != Reasoning::Off)
    }

    /// What a chat may choose for this model: its levels, then Ultra whenever it can reason.
    pub fn choices(&self) -> Vec<Reasoning> {
        let mut choices = self.levels();
        if self.reasons() {
            choices.push(Reasoning::Ultra);
        }
        choices
    }

    /// The effort a chat runs with: `wanted` when the model offers it, else Medium when it lists
    /// Medium, else its lowest level (version 2's rule).
    pub fn pick(&self, wanted: Option<Reasoning>) -> Reasoning {
        let choices = self.choices();
        match wanted {
            Some(wanted) if choices.contains(&wanted) => wanted,
            _ if choices.contains(&Reasoning::Medium) => Reasoning::Medium,
            _ => choices[0],
        }
    }

    /// What is sent to the provider for `level`. Off sends no reasoning setting at all; Ultra
    /// sends the model's highest level.
    pub fn effort_for(&self, level: Reasoning) -> Option<Effort> {
        let level = if level.is_ultra() { self.levels().last().copied().unwrap_or(Reasoning::Off) } else { level };
        level.effort()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub format: WireFormat,
    pub base_url: String,
    pub models: Vec<Model>,
    /// Switched on in Settings › Providers. An off connection keeps its key and models but is
    /// left out of the model menu and never chosen: a chat that used it falls back to another
    /// until it is switched on again. Files saved before the switch existed load as on.
    #[serde(default = "on", skip_serializing_if = "is_on")]
    pub enabled: bool,
}

fn on() -> bool {
    true
}

fn is_on(enabled: &bool) -> bool {
    *enabled
}

pub struct NewConnection {
    pub name: String,
    pub format: WireFormat,
    pub base_url: String,
    pub models: Vec<Model>,
}

/// Which project-level shell command is configured or running.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandKind {
    Build,
    Run,
}

impl CommandKind {
    pub fn name(self) -> &'static str {
        match self {
            CommandKind::Build => "build",
            CommandKind::Run => "run",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            CommandKind::Build => "Build",
            CommandKind::Run => "Run",
        }
    }
}

/// A folder the user works in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub path: PathBuf,
    /// The folder's name, shown in the sidebar.
    pub name: String,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub build_command: Option<String>,
    #[serde(default)]
    pub run_command: Option<String>,
}

impl Project {
    pub fn command(&self, kind: CommandKind) -> Option<&str> {
        match kind {
            CommandKind::Build => self.build_command.as_deref(),
            CommandKind::Run => self.run_command.as_deref(),
        }
    }
}

/// How hard the model should think: one of the provider's own effort levels, or Ultra.
///
/// Off sends no reasoning setting at all. Ultra is WackCode's, never a provider's: it sends the
/// model's highest level and adds an instruction to the system prompt. It is kept out of
/// `wack_llm::Effort` so no wire format can ever send it. The order is low to high.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Reasoning {
    Off,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
    Ultra,
}

impl Reasoning {
    /// Every level a model can list, low to high (Ultra is offered on top, never listed).
    pub const LEVELS: [Reasoning; 7] =
        [Self::Off, Self::Minimal, Self::Low, Self::Medium, Self::High, Self::Xhigh, Self::Max];

    /// The effort the provider is asked for; `None` for Off. Each wire format lowers it to what
    /// it supports. Ultra alone asks for Max: [`Model::effort_for`] gives the model's highest.
    pub fn effort(self) -> Option<Effort> {
        Some(match self {
            Self::Off => return None,
            Self::Minimal => Effort::Minimal,
            Self::Low => Effort::Low,
            Self::Medium => Effort::Medium,
            Self::High => Effort::High,
            Self::Xhigh => Effort::Xhigh,
            Self::Max | Self::Ultra => Effort::Max,
        })
    }

    pub fn is_ultra(self) -> bool {
        self == Self::Ultra
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Minimal => "Minimal",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Xhigh => "Xhigh",
            Self::Max => "Max",
            Self::Ultra => "Ultra",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "off" => Some(Self::Off),
            "ultra" => Some(Self::Ultra),
            _ => Some(match Effort::parse(text)? {
                Effort::Minimal => Self::Minimal,
                Effort::Low => Self::Low,
                Effort::Medium => Self::Medium,
                Effort::High => Self::High,
                Effort::Xhigh => Self::Xhigh,
                Effort::Max => Self::Max,
            }),
        }
    }
}

/// Which model a chat talks to. Any part may be missing: the defaults fill it in.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelChoice {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Reasoning>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Chat {
    pub id: String,
    /// The project's id.
    pub project: String,
    pub title: String,
    /// The session file's name inside the sessions folder.
    pub session: String,
    /// Milliseconds since 1970.
    pub created: u64,
    pub updated: u64,
    #[serde(default)]
    pub choice: ModelChoice,
    /// Build, Plan or Ultra Plan. This is the truth between runs; the session file records only
    /// what the model has been told so far.
    #[serde(default)]
    pub mode: AgentMode,
    /// Where the chat's panes are, as the UI wrote it. The engine keeps it without reading it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<serde_json::Value>,
}

/// How the window was left. The window itself opens on a fresh new chat, as version 2 did, so
/// only what is not about a chat is kept: the sidebar's side, and the project that new chat is in.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Ui {
    #[serde(default)]
    pub sidebar_hidden: bool,
    /// The project the user was last in, so the new chat on launch is in it. Opening Settings
    /// leaves it as it was; a project that no longer exists falls back to the first one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_project: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    connections: Vec<Connection>,
    #[serde(default)]
    projects: Vec<Project>,
    #[serde(default)]
    chats: Vec<Chat>,
    /// The last model the user picked; new chats start with it.
    #[serde(default)]
    defaults: ModelChoice,
    #[serde(default)]
    ui: Ui,
    #[serde(default)]
    appearance: Appearance,
    /// Settings › Tools: Plan and Ultra Plan may change things. Off unless the user lifts it.
    #[serde(default)]
    planning_unrestricted: bool,
    /// Settings › Memory's switches.
    #[serde(default)]
    pub(crate) memory: crate::memory::MemoryConfig,
    /// Settings › Providers › Automatic titles.
    #[serde(default)]
    auto_title: crate::title::AutoTitleConfig,
    /// Settings › Skills' and Settings › Commands' switches.
    #[serde(default)]
    skills: crate::skills::SkillsConfig,
    /// Settings › Integrations: TokenTrail's ledger. Recording is the default, and files saved
    /// before the switch existed load as on.
    #[serde(default = "recording_on")]
    record_usage: bool,
    /// Settings › Web.
    #[serde(default)]
    web: crate::web::WebConfig,
}

fn recording_on() -> bool {
    true
}

/// A connection and model resolved from a [`ModelChoice`], ready to build a run.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub connection: Connection,
    pub model: Model,
    /// Always one of the model's choices ([`Model::pick`]).
    pub effort: Reasoning,
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_millis() as u64)
        .unwrap_or(0)
}

/// An id that sorts by creation time and never repeats within one process, even for two ids
/// made in the same nanosecond.
fn new_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST: AtomicU64 = AtomicU64::new(0);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_nanos() as u64)
        .unwrap_or(0);
    let previous = LAST
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
            Some(stamp.max(last + 1))
        })
        .unwrap_or(0);
    format!("{prefix}-{:x}", stamp.max(previous + 1))
}

/// A chat's title from its first message: the first line, cut at a word near 60 characters.
pub fn title_from(prompt: &str) -> String {
    let line = prompt
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("New chat");
    if line.chars().count() <= 60 {
        return line.to_string();
    }
    let cut: String = line.chars().take(60).collect();
    let at = cut.rfind(' ').filter(|at| *at > 30).unwrap_or(cut.len());
    format!("{}…", cut[..at].trim_end_matches([',', '.', ':', ';', ' ']))
}

pub struct Store {
    folder: PathBuf,
    state: State,
    secrets: SecretStore,
    web_usage: std::sync::Arc<crate::web_usage::WebUsage>,
    /// The usage ledger's writer, shared by every run and the title requests.
    usage: std::sync::Arc<UsageWriter>,
    /// Stands in for `$HOME` where skill folders are looked for, in tests.
    home: Option<PathBuf>,
}

impl Store {
    /// Version 3's folder inside the app's data folder. A debug build uses the dev app's folder,
    /// so nothing under development can touch the data of the installed app.
    pub fn default_folder() -> Result<PathBuf, String> {
        let home = std::env::var_os("HOME").ok_or("Could not find your home folder.")?;
        let app = if cfg!(debug_assertions) {
            "com.wackcode.desktop.dev"
        } else {
            "com.wackcode.desktop"
        };
        Ok(Path::new(&home)
            .join("Library/Application Support")
            .join(app)
            .join("v3"))
    }

    pub fn open(folder: &Path) -> Result<Self, String> {
        fs::create_dir_all(folder)
            .map_err(|error| format!("Could not create WackCode's data folder: {error}"))?;
        let _ = fs::set_permissions(folder, fs::Permissions::from_mode(0o700));
        let path = folder.join("state.json");
        let state = if path.exists() {
            let content = fs::read_to_string(&path)
                .map_err(|error| format!("Could not read WackCode's settings: {error}"))?;
            serde_json::from_str(&content)
                .map_err(|error| format!("WackCode's settings file is damaged: {error}"))?
        } else {
            State::default()
        };
        let mut store = Self {
            folder: folder.to_path_buf(),
            usage: std::sync::Arc::new(UsageWriter::new(&root_for(folder), state.record_usage)),
            state,
            secrets: SecretStore::load(folder)?,
            web_usage: std::sync::Arc::new(crate::web_usage::WebUsage::load(folder)),
            home: None,
        };
        store.tend_pictures();
        Ok(store)
    }

    /// Pictures whose copy has gone are let go of; one imported before thumbnails were made gets
    /// one; and one imported before pictures were measured is measured once, now. A picture that cannot be read stays unmeasured, and the theme then
    /// allows for anything behind the text.
    fn tend_pictures(&mut self) {
        let before = self.state.appearance.clone();
        let gone: Vec<String> = before
            .pictures
            .iter()
            .filter(|picture| !self.backdrop_path(picture).exists())
            .map(|picture| picture.file.clone())
            .collect();
        for file in gone {
            self.state.appearance.remove(&file);
        }
        for picture in &self.state.appearance.pictures {
            if !self.backdrop_thumb_path(picture).exists() {
                let _ = crate::appearance::thumbnail(&self.backdrop_path(picture));
            }
        }
        let unmeasured: Vec<(usize, Result<[u8; 2], String>)> = self
            .state
            .appearance
            .pictures
            .iter()
            .enumerate()
            .filter(|(_, picture)| picture.shade.is_none())
            .map(|(index, picture)| {
                let soft = self.backdrop_soft_path(picture);
                (index, crate::appearance::measure(&soft).or_else(|_| crate::appearance::measure(&self.backdrop_path(picture))))
            })
            .collect();
        for (index, shade) in unmeasured {
            if let Ok(shade) = shade {
                self.state.appearance.pictures[index].shade = Some(shade);
            }
        }
        if self.state.appearance != before {
            let _ = self.save();
        }
    }

    fn save(&self) -> Result<(), String> {
        let failed = |error: std::io::Error| format!("Could not save WackCode's settings: {error}");
        let path = self.folder.join("state.json");
        let temporary = path.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(&self.state)
            .map_err(|error| format!("Could not save WackCode's settings: {error}"))?;
        fs::write(&temporary, json).map_err(failed)?;
        fs::rename(&temporary, &path).map_err(failed)
    }

    pub(crate) fn folder(&self) -> &Path {
        &self.folder
    }

    pub(crate) fn save_state(&self) -> Result<(), String> {
        self.save()
    }

    pub(crate) fn web_state(&self) -> &crate::web::WebConfig {
        &self.state.web
    }

    pub(crate) fn web_state_mut(&mut self) -> &mut crate::web::WebConfig {
        &mut self.state.web
    }

    /// A stored secret, for the engine's own clients to hold (`web.rs`). Never handed out.
    pub(crate) fn secret(&self, id: &str) -> Option<&str> {
        self.secrets.get(id)
    }

    pub(crate) fn set_secret(&mut self, id: &str, value: &str) -> Result<(), String> {
        self.secrets.set(id, value)
    }

    pub(crate) fn remove_secret(&mut self, id: &str) -> Result<(), String> {
        self.secrets.remove(id)
    }

    pub(crate) fn web_usage_meter(&self) -> &std::sync::Arc<crate::web_usage::WebUsage> {
        &self.web_usage
    }

    pub(crate) fn memory_config_mut(&mut self) -> &mut crate::memory::MemoryConfig {
        &mut self.state.memory
    }

    pub(crate) fn memory_config(&self) -> &crate::memory::MemoryConfig {
        &self.state.memory
    }

    pub(crate) fn skills_config_mut(&mut self) -> &mut crate::skills::SkillsConfig {
        &mut self.state.skills
    }

    pub(crate) fn skills_config(&self) -> &crate::skills::SkillsConfig {
        &self.state.skills
    }

    pub(crate) fn home_override(&self) -> Option<&PathBuf> {
        self.home.as_ref()
    }

    /// Looks for skill folders under `home` instead of `$HOME`. For tests and scenes.
    pub fn set_home_override(&mut self, home: PathBuf) {
        self.home = Some(home);
    }

    pub fn sessions_folder(&self) -> PathBuf {
        self.folder.join("sessions")
    }

    pub fn connections(&self) -> &[Connection] {
        &self.state.connections
    }

    /// A connection by its name (any letter case) or id.
    pub fn connection(&self, name: &str) -> Option<&Connection> {
        self.state
            .connections
            .iter()
            .find(|connection| connection.id == name || connection.name.eq_ignore_ascii_case(name))
    }

    pub fn add_connection(&mut self, new: NewConnection, key: &str) -> Result<Connection, String> {
        let key = key.trim();
        let format = new.format;
        let (name, base_url, models) = self.clean_connection(None, new)?;
        if key.is_empty() {
            return Err("Enter the API key.".to_string());
        }
        let connection = Connection {
            id: new_id("connection"),
            name,
            format,
            base_url,
            models,
            enabled: true,
        };
        // The key first: a connection listed without its key would fail on first use.
        self.secrets.set(&connection.id, key)?;
        self.state.connections.push(connection.clone());
        self.save()?;
        Ok(connection)
    }

    /// Changes a saved connection in place, keeping its id (so chats that chose it keep it) and
    /// whether it is switched on. A new key replaces the saved one; with none, the saved key
    /// stays, and is never read to compare with anything.
    pub fn update_connection(
        &mut self,
        id: &str,
        new: NewConnection,
        key: Option<&str>,
    ) -> Result<Connection, String> {
        let index = self
            .state
            .connections
            .iter()
            .position(|connection| connection.id == id)
            .ok_or("That connection is no longer there.")?;
        let format = new.format;
        let (name, base_url, models) = self.clean_connection(Some(id), new)?;
        if let Some(key) = key.map(str::trim).filter(|key| !key.is_empty()) {
            self.secrets.set(id, key)?;
        }
        let connection = &mut self.state.connections[index];
        connection.name = name;
        connection.format = format;
        connection.base_url = base_url;
        connection.models = models;
        let saved = connection.clone();
        self.save()?;
        Ok(saved)
    }

    /// A connection's fields as they are saved: trimmed, the address's trailing slash gone, and
    /// at least one model. `except` is the connection being edited, which may keep its name.
    fn clean_connection(
        &self,
        except: Option<&str>,
        new: NewConnection,
    ) -> Result<(String, String, Vec<Model>), String> {
        let name = new.name.trim().to_string();
        let base_url = new.base_url.trim().trim_end_matches('/').to_string();
        if name.is_empty() {
            return Err("Give the connection a name.".to_string());
        }
        if self.state.connections.iter().any(|connection| {
            Some(connection.id.as_str()) != except
                && (connection.id == name || connection.name.eq_ignore_ascii_case(&name))
        }) {
            return Err(format!("There is already a connection named {name}."));
        }
        if !(base_url.starts_with("https://") || base_url.starts_with("http://")) {
            return Err("The address must start with https:// or http://.".to_string());
        }
        let models: Vec<Model> = new
            .models
            .into_iter()
            .map(|model| Model {
                id: model.id.trim().to_string(),
                ..model
            })
            .filter(|model| !model.id.is_empty())
            .collect();
        if models.is_empty() {
            return Err("Add at least one model.".to_string());
        }
        Ok((name, base_url, models))
    }

    /// Switches a connection on or off. Its key, its models and every chat's choice of it stay as
    /// they are, so switching it back on restores them.
    pub fn set_connection_enabled(&mut self, id: &str, enabled: bool) -> Result<(), String> {
        let connection = self
            .state
            .connections
            .iter_mut()
            .find(|connection| connection.id == id)
            .ok_or("That connection is no longer there.")?;
        connection.enabled = enabled;
        self.save()
    }

    /// The connections switched on: the ones a chat may use.
    pub fn enabled_connections(&self) -> impl Iterator<Item = &Connection> {
        self.state.connections.iter().filter(|connection| connection.enabled)
    }

    /// Whether a key is kept for the connection. The key itself is not handed out here.
    pub fn has_key(&self, id: &str) -> bool {
        self.secrets.get(id).is_some()
    }

    /// A connection's saved key, for the eye in Settings › Providers: the one deliberate way
    /// a key leaves the store to be read. The page shows it until the field is masked again,
    /// and only a connection's own key can be asked for; search providers have no such path.
    pub fn reveal_connection_key(&self, id: &str) -> Result<String, String> {
        let connection = self
            .state
            .connections
            .iter()
            .find(|connection| connection.id == id)
            .ok_or("That connection is no longer there.")?;
        self.secrets
            .get(&connection.id)
            .map(str::to_string)
            .ok_or_else(|| format!("No key is saved for {}.", connection.name))
    }

    pub fn remove_connection(&mut self, id: &str) -> Result<(), String> {
        let borrowing = self.search_providers_borrowing(id);
        if !borrowing.is_empty() {
            return Err(format!(
                "{} uses this connection's key for web search. Give it its own key in Settings › Web first.",
                borrowing.join(", ")
            ));
        }
        self.state
            .connections
            .retain(|connection| connection.id != id);
        self.save()?;
        self.secrets.remove(id)
    }

    pub fn projects(&self) -> &[Project] {
        &self.state.projects
    }

    pub fn project(&self, id: &str) -> Option<&Project> {
        self.state.projects.iter().find(|project| project.id == id)
    }

    /// Adds a folder as a project, or returns the project it already is.
    pub fn add_project(&mut self, path: &Path) -> Result<Project, String> {
        let path = path
            .canonicalize()
            .map_err(|error| format!("Could not open {}: {error}", path.display()))?;
        if !path.is_dir() {
            return Err(format!("{} is not a folder.", path.display()));
        }
        if let Some(existing) = self
            .state
            .projects
            .iter()
            .find(|project| project.path == path)
        {
            return Ok(existing.clone());
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let project = Project {
            id: new_id("project"),
            path,
            name,
            pinned: false,
            build_command: None,
            run_command: None,
        };
        self.state.projects.push(project.clone());
        self.save()?;
        Ok(project)
    }

    /// Takes a project and its chats off the list. Their session files stay on disk.
    pub fn remove_project(&mut self, id: &str) -> Result<(), String> {
        self.state.projects.retain(|project| project.id != id);
        self.state.chats.retain(|chat| chat.project != id);
        self.save()
    }

    pub fn set_pinned(&mut self, id: &str, pinned: bool) -> Result<(), String> {
        let project = self
            .state
            .projects
            .iter_mut()
            .find(|project| project.id == id)
            .ok_or("That project is gone.")?;
        project.pinned = pinned;
        self.save()
    }

    /// Saves or clears a project's Build or Run command. Trims whitespace and rejects null bytes.
    pub fn set_project_command(
        &mut self,
        id: &str,
        kind: CommandKind,
        command: Option<&str>,
    ) -> Result<Project, String> {
        let cleaned = match command {
            Some(raw) => {
                if raw.contains('\0') {
                    return Err("The command cannot contain null characters.".into());
                }
                let trimmed = raw.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            }
            None => None,
        };
        let updated = {
            let project = self
                .state
                .projects
                .iter_mut()
                .find(|project| project.id == id)
                .ok_or("That project is gone.")?;
            match kind {
                CommandKind::Build => project.build_command = cleaned,
                CommandKind::Run => project.run_command = cleaned,
            }
            project.clone()
        };
        self.save()?;
        Ok(updated)
    }

    /// A project's chats, most recently used first.
    pub fn chats(&self, project: &str) -> Vec<&Chat> {
        let mut chats: Vec<&Chat> = self
            .state
            .chats
            .iter()
            .filter(|chat| chat.project == project)
            .collect();
        chats.sort_by_key(|chat| std::cmp::Reverse(chat.updated));
        chats
    }

    pub fn chat(&self, id: &str) -> Option<&Chat> {
        self.state.chats.iter().find(|chat| chat.id == id)
    }

    /// The project of the chat used most recently: where a fresh launch opens its new chat
    /// when nothing else remembers a project.
    pub fn last_used_project(&self) -> Option<&Project> {
        self.state
            .chats
            .iter()
            .max_by_key(|chat| chat.updated)
            .and_then(|chat| self.project(&chat.project))
    }

    /// Lists a new chat. Its session file is created by its first run.
    pub fn create_chat(
        &mut self,
        project: &str,
        title: &str,
        choice: ModelChoice,
        mode: AgentMode,
    ) -> Result<Chat, String> {
        if self.project(project).is_none() {
            return Err("That project is gone.".to_string());
        }
        let id = new_id("chat");
        let now = now_millis();
        let chat = Chat {
            session: format!("{id}.jsonl"),
            id,
            project: project.to_string(),
            title: title.to_string(),
            created: now,
            updated: now,
            choice,
            mode,
            layout: None,
        };
        self.state.chats.push(chat.clone());
        self.save()?;
        Ok(chat)
    }

    pub fn session_path(&self, chat: &Chat) -> PathBuf {
        self.sessions_folder().join(&chat.session)
    }

    /// Where a chat's background commands write their output (`v3/tasks/<chat>/`).
    pub fn tasks_folder(&self, chat: &Chat) -> PathBuf {
        self.folder.join("tasks").join(&chat.id)
    }

    /// Marks a chat as just used, so it moves to the top of its project.
    pub fn touch_chat(&mut self, id: &str) -> Result<(), String> {
        if let Some(chat) = self.state.chats.iter_mut().find(|chat| chat.id == id) {
            chat.updated = now_millis();
            self.save()?;
        }
        Ok(())
    }

    pub fn rename_chat(&mut self, id: &str, title: &str) -> Result<(), String> {
        let title = title.trim();
        if title.is_empty() {
            return Err("Give the chat a name.".to_string());
        }
        let chat = self
            .state
            .chats
            .iter_mut()
            .find(|chat| chat.id == id)
            .ok_or("That chat is gone.")?;
        chat.title = title.to_string();
        self.save()
    }

    /// Switches a chat between Build, Plan and Ultra Plan. It applies from the chat's next run.
    pub fn set_mode(&mut self, id: &str, mode: AgentMode) -> Result<(), String> {
        let chat = self.state.chats.iter_mut().find(|chat| chat.id == id).ok_or("That chat is gone.")?;
        if chat.mode == mode {
            return Ok(());
        }
        chat.mode = mode;
        self.save()
    }

    /// Whether Plan and Ultra Plan may change things (Settings › Tools).
    pub fn planning_unrestricted(&self) -> bool {
        self.state.planning_unrestricted
    }

    pub fn set_planning_unrestricted(&mut self, unrestricted: bool) -> Result<(), String> {
        if self.state.planning_unrestricted == unrestricted {
            return Ok(());
        }
        self.state.planning_unrestricted = unrestricted;
        self.save()
    }

    /// Settings › Providers › Automatic titles, as saved.
    pub fn auto_title(&self) -> &crate::title::AutoTitleConfig {
        &self.state.auto_title
    }

    /// Saves the automatic-title setting. Switching titles on needs a model that can be used now;
    /// switching them off, or picking a model while off, always works.
    pub fn set_auto_title(&mut self, config: crate::title::AutoTitleConfig) -> Result<(), String> {
        if config.enabled {
            self.title_model(&config)?;
        }
        if self.state.auto_title == config {
            return Ok(());
        }
        self.state.auto_title = config;
        self.save()
    }

    /// The connection and model a title setting names, if both still exist and the connection is
    /// on.
    fn title_model(&self, config: &crate::title::AutoTitleConfig) -> Result<(Connection, Model), String> {
        let (Some(connection), Some(model)) = (config.connection.as_deref(), config.model.as_deref()) else {
            return Err("Choose a model for automatic titles.".into());
        };
        let connection = self
            .connections()
            .iter()
            .find(|listed| listed.id == connection)
            .ok_or("The title model's connection is gone. Choose another model.")?;
        if !connection.enabled {
            return Err(format!("Switch on {} before using it for automatic titles.", connection.name));
        }
        let listed = connection
            .models
            .iter()
            .find(|listed| listed.id == model)
            .ok_or_else(|| format!("{} no longer lists {model}. Choose another model.", connection.name))?;
        Ok((connection.clone(), listed.clone()))
    }

    /// What a new chat's title request needs: `None` while titles are off, an error when their
    /// model can no longer be used, else the provider, the connection (whose id and name the
    /// ledger records) and the model to ask.
    pub fn title_setup(&self) -> Option<Result<(HttpProvider, Connection, Model), String>> {
        let config = &self.state.auto_title;
        if !config.enabled {
            return None;
        }
        Some(self.title_model(config).and_then(|(connection, model)| Ok((self.provider(&connection)?, connection, model))))
    }

    /// Settings › Integrations: whether TokenTrail's ledger is being recorded.
    pub fn usage_recording(&self) -> bool {
        self.state.record_usage
    }

    /// The ledger's writer.
    pub fn usage(&self) -> &std::sync::Arc<UsageWriter> {
        &self.usage
    }

    /// Turns recording on or off. Off gives up on everything still queued, counting it.
    pub fn set_usage_recording(&mut self, enabled: bool) -> Result<(), String> {
        if self.state.record_usage == enabled {
            return Ok(());
        }
        self.state.record_usage = enabled;
        self.usage.set_enabled(enabled);
        self.save()
    }

    /// The ledger's recorder for a run of `session` (a chat, or a CLI session's stem) working
    /// in `workspace` on `connection`. The project is the workspace's repository root, so
    /// worktrees of one repository land in the same project.
    pub fn usage_recorder(&self, connection: &Connection, session: &str, workspace: &Path) -> std::sync::Arc<RunRecorder> {
        let project = crate::git::common_root(workspace).unwrap_or_else(|| workspace.to_path_buf());
        std::sync::Arc::new(RunRecorder::new(&self.usage, connection, session, &project, workspace))
    }

    pub fn set_layout(&mut self, id: &str, layout: Option<serde_json::Value>) -> Result<(), String> {
        let chat = self.state.chats.iter_mut().find(|chat| chat.id == id).ok_or("That chat is gone.")?;
        if chat.layout == layout {
            return Ok(());
        }
        chat.layout = layout;
        self.save()
    }

    pub fn appearance(&self) -> &Appearance {
        &self.state.appearance
    }

    /// Changes how the app looks. Every picture must be one
    /// [`crate::appearance::import_backdrop`] wrote into [`Store::backgrounds_folder`]. Copies no
    /// picture in the library uses are deleted.
    pub fn set_appearance(&mut self, appearance: Appearance) -> Result<(), String> {
        let appearance = appearance.checked()?;
        if appearance.pictures.iter().any(|picture| !self.backdrop_path(picture).exists()) {
            return Err("A backdrop’s picture is missing. Choose it again.".into());
        }
        if self.state.appearance == appearance {
            return Ok(());
        }
        self.state.appearance = appearance;
        self.save()?;
        let keep: Vec<&str> = self.state.appearance.pictures.iter().map(|picture| picture.file.as_str()).collect();
        crate::appearance::tidy_backgrounds(&self.backgrounds_folder(), &keep);
        Ok(())
    }

    /// The small sharp copy Settings shows a picture as.
    pub fn backdrop_thumb_path(&self, backdrop: &Backdrop) -> PathBuf {
        self.backgrounds_folder().join(crate::appearance::thumb_name(&backdrop.file))
    }

    pub fn backgrounds_folder(&self) -> PathBuf {
        self.folder.join("backgrounds")
    }

    pub fn backdrop_path(&self, backdrop: &Backdrop) -> PathBuf {
        self.backgrounds_folder().join(&backdrop.file)
    }

    /// The softened copy drawn behind a conversation ([`crate::appearance::soften`]).
    pub fn backdrop_soft_path(&self, backdrop: &Backdrop) -> PathBuf {
        self.backgrounds_folder().join(crate::appearance::soft_name(&backdrop.file))
    }

    pub fn ui(&self) -> &Ui {
        &self.state.ui
    }

    pub fn set_ui(&mut self, ui: Ui) -> Result<(), String> {
        if self.state.ui == ui {
            return Ok(());
        }
        self.state.ui = ui;
        self.save()
    }

    /// Deletes a chat and its conversation file.
    pub fn delete_chat(&mut self, id: &str) -> Result<(), String> {
        let Some(chat) = self.chat(id).cloned() else {
            return Ok(());
        };
        self.state.chats.retain(|listed| listed.id != id);
        self.save()?;
        let _ = fs::remove_dir_all(self.tasks_folder(&chat));
        match fs::remove_file(self.session_path(&chat)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("Could not delete the chat's file: {error}"))
            }
            _ => Ok(()),
        }
    }

    /// Sets a chat's model, or the default for new chats when `chat` is `None`. Picking a model
    /// for a chat also makes it the default, since it is the user's latest choice.
    pub fn set_choice(&mut self, chat: Option<&str>, choice: ModelChoice) -> Result<(), String> {
        if let Some(id) = chat
            && let Some(listed) = self.state.chats.iter_mut().find(|listed| listed.id == id)
        {
            listed.choice = choice.clone();
        }
        self.state.defaults = choice;
        self.save()
    }

    pub fn defaults(&self) -> &ModelChoice {
        &self.state.defaults
    }

    /// The connection and model a choice means today. A missing, removed or switched-off
    /// connection falls back to the default and then to the first one switched on; a missing model
    /// to the connection's first. With every connection off there is nothing to use.
    pub fn resolve(&self, choice: &ModelChoice) -> Option<Resolved> {
        let defaults = &self.state.defaults;
        let connection = [choice.connection.as_deref(), defaults.connection.as_deref()]
            .into_iter()
            .flatten()
            .find_map(|id| self.enabled_connections().find(|connection| connection.id == id))
            .or_else(|| self.enabled_connections().next())?;
        let model = [choice.model.as_deref(), defaults.model.as_deref()]
            .into_iter()
            .flatten()
            .find_map(|id| connection.models.iter().find(|model| model.id == id))
            .or_else(|| connection.models.first())?;
        let effort = if choice.connection.is_some() || choice.model.is_some() {
            choice.effort
        } else {
            defaults.effort
        };
        Some(Resolved {
            connection: connection.clone(),
            effort: model.pick(effort),
            model: model.clone(),
        })
    }

    /// The provider that sends this connection's requests. A key comes out of the secret store
    /// here and goes straight into the provider that needs it; `reveal_connection_key` is the
    /// only way it comes out to be read.
    pub fn provider(&self, connection: &Connection) -> Result<HttpProvider, String> {
        let key = self.secrets.get(&connection.id).ok_or_else(|| {
            format!(
                "The connection {} has no API key. Add it again.",
                connection.name
            )
        })?;
        HttpProvider::new(connection.format, &connection.base_url, key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new(name: &str) -> NewConnection {
        NewConnection {
            name: name.into(),
            format: WireFormat::OpenaiChat,
            base_url: "https://llm.example/v1/".into(),
            models: vec![
                Model::new(" big-model ", Some(200_000)),
                Model::new("", None),
            ],
        }
    }

    #[test]
    fn a_connection_is_saved_with_its_key_kept_apart() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        let added = store
            .add_connection(new("Testing"), " sk-secret-value \n")
            .unwrap();
        assert_eq!(added.base_url, "https://llm.example/v1");
        assert_eq!(
            added.models,
            [Model::new("big-model", Some(200_000))]
        );

        let state = fs::read_to_string(folder.path().join("state.json")).unwrap();
        assert!(state.contains("Testing") && state.contains("openai-chat"));
        assert!(
            !state.contains("sk-secret-value"),
            "the key must never reach state.json"
        );

        let reopened = Store::open(folder.path()).unwrap();
        let found = reopened.connection("testing").unwrap();
        assert_eq!(*found, added);
        assert!(reopened.provider(found).is_ok());
    }

    #[test]
    fn projects_are_added_once_pinned_and_removed_with_their_chats() {
        let folder = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        let project = store.add_project(work.path()).unwrap();
        assert_eq!(
            store.add_project(&work.path().join(".")).unwrap().id,
            project.id,
            "the same folder is one project"
        );
        assert!(store.add_project(&work.path().join("missing")).is_err());

        store.set_pinned(&project.id, true).unwrap();
        let first = store
            .create_chat(&project.id, "First", ModelChoice::default(), AgentMode::Build)
            .unwrap();
        let second = store
            .create_chat(&project.id, "Second", ModelChoice::default(), AgentMode::Build)
            .unwrap();
        assert_ne!(first.id, second.id);
        store.touch_chat(&first.id).unwrap();
        let titles: Vec<&str> = store
            .chats(&project.id)
            .iter()
            .map(|chat| chat.title.as_str())
            .collect();
        assert_eq!(
            titles,
            ["First", "Second"],
            "the chat used last comes first"
        );

        let reopened = Store::open(folder.path()).unwrap();
        assert!(reopened.project(&project.id).unwrap().pinned);
        assert_eq!(reopened.chats(&project.id).len(), 2);
        assert_eq!(store.last_used_project().map(|project| project.id.as_str()), Some(project.id.as_str()));

        store.remove_project(&project.id).unwrap();
        assert!(store.projects().is_empty());
        assert!(store.chat(&first.id).is_none());
    }

    #[test]
    fn deleting_a_chat_deletes_its_file() {
        let folder = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        let project = store.add_project(work.path()).unwrap();
        let chat = store
            .create_chat(&project.id, "Doomed", ModelChoice::default(), AgentMode::Build)
            .unwrap();
        let path = store.session_path(&chat);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{}\n").unwrap();
        store.delete_chat(&chat.id).unwrap();
        assert!(!path.exists());
        assert!(store.chat(&chat.id).is_none());
    }

    #[test]
    fn a_choice_resolves_through_the_defaults() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        assert_eq!(
            store.resolve(&ModelChoice::default()),
            None,
            "nothing to talk to yet"
        );
        let first = store.add_connection(new("First"), "k1").unwrap();
        let mut two_models = new("Second");
        two_models.models = vec![
            Model::new("a", None),
            Model { efforts: vec![Reasoning::Off, Reasoning::Medium, Reasoning::High], ..Model::new("b", Some(64_000)) },
        ];
        let second = store.add_connection(two_models, "k2").unwrap();

        let resolved = store.resolve(&ModelChoice::default()).unwrap();
        assert_eq!(
            resolved.connection.id, first.id,
            "with no default, the first connection"
        );

        let picked = ModelChoice {
            connection: Some(second.id.clone()),
            model: Some("b".into()),
            effort: Some(Reasoning::High),
        };
        store.set_choice(None, picked).unwrap();
        let resolved = store.resolve(&ModelChoice::default()).unwrap();
        assert_eq!(
            (
                resolved.model.id.as_str(),
                resolved.model.context_window,
                resolved.effort
            ),
            ("b", Some(64_000), Reasoning::High)
        );

        let gone = ModelChoice {
            connection: Some("connection-gone".into()),
            model: Some("zzz".into()),
            effort: None,
        };
        let resolved = store.resolve(&gone).unwrap();
        assert_eq!(
            resolved.connection.id, second.id,
            "a removed connection falls back to the default"
        );
        assert_eq!(resolved.model.id, "b");
    }

    #[test]
    fn a_connection_switched_off_is_skipped_and_kept() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        let first = store.add_connection(new("First"), "k1").unwrap();
        let second = store.add_connection(new("Second"), "k2").unwrap();
        let project = tempfile::tempdir().unwrap();
        let project = store.add_project(project.path()).unwrap();
        let picked = ModelChoice {
            connection: Some(first.id.clone()),
            model: None,
            effort: None,
        };
        let chat = store
            .create_chat(&project.id, "Uses first", picked.clone(), AgentMode::Build)
            .unwrap();

        store.set_connection_enabled(&first.id, false).unwrap();
        assert_eq!(store.resolve(&picked).unwrap().connection.id, second.id, "off falls back");
        assert_eq!(store.enabled_connections().count(), 1);
        assert!(store.has_key(&first.id), "off keeps the key");
        assert_eq!(store.chat(&chat.id).unwrap().choice, picked, "the chat's choice is kept");

        let reopened = Store::open(folder.path()).unwrap();
        assert!(!reopened.connection("first").unwrap().enabled, "off is saved");

        store.set_connection_enabled(&second.id, false).unwrap();
        assert_eq!(store.resolve(&picked), None, "with every connection off, nothing to use");

        store.set_connection_enabled(&first.id, true).unwrap();
        assert_eq!(store.resolve(&picked).unwrap().connection.id, first.id, "on again, chosen again");
    }

    #[test]
    fn a_connection_saved_before_the_switch_loads_as_on() {
        let saved = r#"{"id":"c","name":"Old","format":"openai-chat","base_url":"https://x","models":[]}"#;
        let connection: Connection = serde_json::from_str(saved).unwrap();
        assert!(connection.enabled);
        assert!(!serde_json::to_string(&connection).unwrap().contains("enabled"), "on is the default, not written");
    }

    #[test]
    fn mode_reasoning_and_the_planning_setting_are_saved() {
        let folder = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        let project = store.add_project(work.path()).unwrap();
        let choice = ModelChoice { effort: Some(Reasoning::Ultra), ..ModelChoice::default() };
        let chat = store.create_chat(&project.id, "Plan it", choice, AgentMode::Plan).unwrap();
        store.set_mode(&chat.id, AgentMode::UltraPlan).unwrap();
        store.set_planning_unrestricted(true).unwrap();

        let store = Store::open(folder.path()).unwrap();
        let chat = store.chat(&chat.id).unwrap();
        assert_eq!(chat.mode, AgentMode::UltraPlan);
        assert_eq!(chat.choice.effort, Some(Reasoning::Ultra));
        assert!(store.planning_unrestricted());

        // Ultra is the top effort plus an instruction; the provider never hears the word.
        assert_eq!(Reasoning::Ultra.effort(), Some(Effort::Max));
        assert_eq!(Reasoning::Off.effort(), None);
        assert_eq!(Reasoning::parse("off"), Some(Reasoning::Off));
        assert_eq!(Reasoning::parse("minimal"), Some(Reasoning::Minimal));
        assert_eq!(Reasoning::parse("ultra"), Some(Reasoning::Ultra));
        assert_eq!(Reasoning::parse("xhigh"), Some(Reasoning::Xhigh));
        assert_eq!(serde_json::to_string(&Reasoning::Xhigh).unwrap(), "\"xhigh\"");
        // A chat saved before modes existed is a Build chat.
        let old: Chat = serde_json::from_str(r#"{"id":"c","project":"p","title":"t","session":"c.jsonl","created":1,"updated":1,"choice":{"effort":"high"}}"#).unwrap();
        assert_eq!((old.mode, old.choice.effort), (AgentMode::Build, Some(Reasoning::High)));
    }

    #[test]
    fn titles_come_from_the_first_line() {
        assert_eq!(
            title_from("\n  Fix the decoder\nand more"),
            "Fix the decoder"
        );
        let long =
            "Make retries honour Retry-After when the server sends it, in seconds or as a date";
        let title = title_from(long);
        assert!(
            title.ends_with('…') && title.chars().count() <= 61,
            "{title}"
        );
        assert_eq!(title_from("   "), "New chat");
    }

    #[test]
    fn bad_connections_are_refused_and_removal_forgets_the_key() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        assert!(store.add_connection(new("Testing"), "").is_err());
        assert!(
            store
                .add_connection(
                    NewConnection {
                        base_url: "llm.example".into(),
                        ..new("A")
                    },
                    "k"
                )
                .is_err()
        );
        assert!(
            store
                .add_connection(
                    NewConnection {
                        models: vec![],
                        ..new("B")
                    },
                    "k"
                )
                .is_err()
        );
        assert!(store.connections().is_empty());

        let added = store
            .add_connection(new("Testing"), "sk-secret-value")
            .unwrap();
        assert!(
            store
                .add_connection(new("TESTING"), "k")
                .unwrap_err()
                .contains("already")
        );
        store.remove_connection(&added.id).unwrap();
        assert!(store.connections().is_empty());
        assert!(
            !fs::read_to_string(folder.path().join("secrets.json"))
                .unwrap()
                .contains("sk-secret-value")
        );
    }

    #[test]
    fn a_connection_is_updated_in_place_and_keeps_its_key() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        let added = store.add_connection(new("Testing"), "sk-first").unwrap();
        store.set_connection_enabled(&added.id, false).unwrap();

        let updated = store
            .update_connection(
                &added.id,
                NewConnection {
                    name: "Testing renamed".into(),
                    format: WireFormat::AnthropicMessages,
                    base_url: "https://other.example/v1/".into(),
                    models: vec![Model::new(" new-model ", Some(64_000))],
                },
                None,
            )
            .unwrap();
        assert_eq!(updated.id, added.id, "the id is what chats chose");
        assert!(!updated.enabled, "the switch is left as it was");
        assert_eq!(updated.name, "Testing renamed");
        assert_eq!(updated.base_url, "https://other.example/v1");
        assert_eq!(updated.models[0].id, "new-model");
        assert!(store.has_key(&added.id), "no key given keeps the saved one");
        assert!(store.provider(&updated).is_ok());

        let second = store.add_connection(new("Second"), "sk-second").unwrap();
        assert!(
            store
                .update_connection(
                    &second.id,
                    NewConnection {
                        name: "TESTING RENAMED".into(),
                        ..new("Ignored")
                    },
                    None
                )
                .unwrap_err()
                .contains("already")
        );
        assert!(store.update_connection("missing", new("X"), None).is_err());

        let with_new_key = store
            .update_connection(&added.id, new("Testing"), Some(" sk-new "))
            .unwrap();
        assert!(store.provider(&with_new_key).is_ok());
        let secrets = fs::read_to_string(folder.path().join("secrets.json")).unwrap();
        assert!(secrets.contains("sk-new") && !secrets.contains("sk-first"));
    }

    #[test]
    fn a_connection_key_is_revealed_only_when_asked_for() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        let added = store.add_connection(new("Testing"), "sk-reveal-me").unwrap();
        assert_eq!(store.reveal_connection_key(&added.id).unwrap(), "sk-reveal-me");
        assert_eq!(store.reveal_connection_key("missing").unwrap_err(), "That connection is no longer there.");

        // A connection switched off keeps its key, so the eye still has something to show.
        store.set_connection_enabled(&added.id, false).unwrap();
        assert_eq!(store.reveal_connection_key(&added.id).unwrap(), "sk-reveal-me");

        // Removing the connection forgets the key with it.
        store.remove_connection(&added.id).unwrap();
        assert!(store.reveal_connection_key(&added.id).is_err());

        // A search provider's key is never reachable this way, whatever id it is given.
        use crate::web::{SearchDraft, search::SearchKind};
        let search = store.save_search_provider(&SearchDraft::new(SearchKind::Brave), Some("bk-search-key")).unwrap();
        assert!(store.reveal_connection_key(&search).is_err());
    }

    #[test]
    fn appearance_is_kept_and_unused_pictures_are_deleted() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        let import = |name: &str, colour: [u8; 3], store: &Store| {
            let picture = folder.path().join(name);
            image::RgbImage::from_pixel(40, 20, image::Rgb(colour)).save(&picture).unwrap();
            crate::appearance::import_backdrop(&picture, &store.backgrounds_folder()).unwrap()
        };
        let first = import("sky.png", [30, 90, 200], &store);
        let second = import("sand.png", [200, 170, 90], &store);
        let mut appearance = Appearance { mode: crate::Mode::Dark, theme: crate::ThemeName::Rose, ..Appearance::default() };
        appearance.add(first.clone());
        appearance.add(second.clone());
        store.set_appearance(appearance).unwrap();

        let reopened = Store::open(folder.path()).unwrap();
        assert_eq!(reopened.appearance().backdrop().map(|picture| picture.file.as_str()), Some(second.file.as_str()));
        assert_eq!(reopened.appearance().theme, crate::ThemeName::Rose);
        let mut hidden = store.appearance().clone();
        hidden.show(None);
        store.set_appearance(hidden).unwrap();
        assert!(store.backdrop_path(&first).exists() && store.backdrop_path(&second).exists(), "a picture not shown is kept");

        let mut fewer = store.appearance().clone();
        fewer.remove(&first.file);
        store.set_appearance(fewer).unwrap();
        assert!(!store.backdrop_path(&first).exists(), "the copy goes with the picture");
        assert!(!store.backgrounds_folder().join(crate::appearance::soft_name(&first.file)).exists());
        assert!(!store.backdrop_thumb_path(&first).exists());
        let mut missing = Appearance::default();
        missing.add(first);
        assert!(store.set_appearance(missing).is_err(), "a picture that is gone is refused");

        // A copy deleted behind WackCode's back is let go of when the store opens.
        std::fs::remove_file(store.backdrop_path(&second)).unwrap();
        assert!(Store::open(folder.path()).unwrap().appearance().pictures.is_empty());
    }

    #[test]
    fn a_picture_imported_before_measuring_is_measured_on_open() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        let picture = folder.path().join("grey.png");
        image::RgbImage::from_pixel(40, 20, image::Rgb([119, 119, 119])).save(&picture).unwrap();
        let backdrop = crate::appearance::import_backdrop(&picture, &store.backgrounds_folder()).unwrap();
        let unmeasured = Backdrop { shade: None, ..backdrop.clone() };
        let mut appearance = Appearance::default();
        appearance.add(unmeasured);
        store.set_appearance(appearance).unwrap();

        std::fs::remove_file(store.backdrop_thumb_path(&backdrop)).unwrap();
        let reopened = Store::open(folder.path()).unwrap();
        assert!(reopened.backdrop_thumb_path(&backdrop).exists(), "a picture from before thumbnails gets one on open");
        let [dark, light] = reopened.appearance().backdrop().unwrap().shade.expect("measured on open");
        assert!(dark.abs_diff(119) <= 2 && light.abs_diff(119) <= 2, "{dark} to {light}");
        let again = Store::open(folder.path()).unwrap();
        assert!(again.appearance().backdrop().unwrap().shade.is_some(), "and the measure is saved");
    }

    #[test]
    fn project_commands_are_trimmed_validated_and_persisted() {
        let folder = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        let project = store.add_project(work.path()).unwrap();
        assert_eq!(project.build_command, None);
        assert_eq!(project.run_command, None);

        let updated = store
            .set_project_command(&project.id, CommandKind::Build, Some("  cargo test --workspace  "))
            .unwrap();
        assert_eq!(updated.command(CommandKind::Build), Some("cargo test --workspace"));
        assert_eq!(updated.command(CommandKind::Run), None);

        let updated = store
            .set_project_command(&project.id, CommandKind::Run, Some("cargo run\n"))
            .unwrap();
        assert_eq!(updated.command(CommandKind::Run), Some("cargo run"));

        assert!(
            store
                .set_project_command(&project.id, CommandKind::Build, Some("bad\0cmd"))
                .is_err()
        );

        let cleared = store
            .set_project_command(&project.id, CommandKind::Build, Some("   "))
            .unwrap();
        assert_eq!(cleared.command(CommandKind::Build), None);

        let reopened = Store::open(folder.path()).unwrap();
        let persisted = reopened.project(&project.id).unwrap();
        assert_eq!(persisted.command(CommandKind::Build), None);
        assert_eq!(persisted.command(CommandKind::Run), Some("cargo run"));
    }

    #[test]
    fn automatic_titles_are_off_until_a_usable_model_is_chosen() {
        use crate::title::AutoTitleConfig;
        let folder = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        assert_eq!(store.auto_title(), &AutoTitleConfig::default());
        assert!(store.title_setup().is_none(), "off: no request");

        let on_without_model = AutoTitleConfig { enabled: true, ..AutoTitleConfig::default() };
        assert_eq!(store.set_auto_title(on_without_model).unwrap_err(), "Choose a model for automatic titles.");

        let connection = store.add_connection(new("Small"), "key").unwrap();
        let model = connection.models[0].id.clone();
        let wrong = AutoTitleConfig { enabled: true, connection: Some(connection.id.clone()), model: Some("missing".into()) };
        assert!(store.set_auto_title(wrong).is_err());

        let chosen = AutoTitleConfig { enabled: true, connection: Some(connection.id.clone()), model: Some(model.clone()) };
        store.set_auto_title(chosen.clone()).unwrap();
        let (_, _, asked) = store.title_setup().unwrap().unwrap();
        assert_eq!(asked.id, model);
        let reopened = Store::open(folder.path()).unwrap();
        assert_eq!(reopened.auto_title(), &chosen, "saved");

        store.set_connection_enabled(&connection.id, false).unwrap();
        assert!(store.title_setup().unwrap().is_err(), "an off connection can't title");
        store.set_connection_enabled(&connection.id, true).unwrap();
        store.remove_connection(&connection.id).unwrap();
        assert!(store.title_setup().unwrap().is_err(), "a removed connection can't either");

        let off = AutoTitleConfig { enabled: false, ..chosen };
        store.set_auto_title(off).unwrap();
        assert!(store.title_setup().is_none());
    }

    #[test]
    fn a_model_offers_only_the_efforts_it_lists() {
        let plain = Model::new("plain", None);
        assert_eq!(plain.choices(), vec![Reasoning::Off], "listing nothing, it does not reason");
        assert_eq!(plain.pick(Some(Reasoning::High)), Reasoning::Off);
        assert_eq!(plain.effort_for(Reasoning::Off), None);

        let thinking = Model { efforts: vec![Reasoning::High, Reasoning::Off, Reasoning::Low, Reasoning::Ultra], ..Model::new("t", None) };
        assert_eq!(thinking.levels(), vec![Reasoning::Off, Reasoning::Low, Reasoning::High], "sorted, never Ultra");
        assert_eq!(thinking.choices().last(), Some(&Reasoning::Ultra), "Ultra sits on top of a model that reasons");
        assert_eq!(thinking.pick(Some(Reasoning::Low)), Reasoning::Low);
        assert_eq!(thinking.pick(Some(Reasoning::Max)), Reasoning::Off, "no Medium: its lowest");
        assert_eq!(thinking.effort_for(Reasoning::Ultra), Some(Effort::High), "Ultra sends its highest level");

        let medium = Model { efforts: vec![Reasoning::Low, Reasoning::Medium], ..Model::new("m", None) };
        assert_eq!(medium.pick(None), Reasoning::Medium, "Medium when it lists it");

        // Saved before the list existed: no reasoning.
        let old: Model = serde_json::from_str(r#"{"id":"old"}"#).unwrap();
        assert_eq!(old.choices(), vec![Reasoning::Off]);
        assert_eq!(serde_json::to_string(&old).unwrap(), r#"{"id":"old"}"#);
    }
}
