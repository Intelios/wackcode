//! Settings › MCP servers: the servers the user added, where each is available (areas and
//! projects), and what a run is given. The client itself is `wack_mcp`. Ported from version 2's
//! `mcp.rs`, with its validation and tests; scoping a server by area and projects is new.
//!
//! Invariants:
//! - `state.json` keeps each server with its header and environment variable *names* only. Their
//!   values, often tokens, live in `secrets.json` under `mcp:<id>` as `{headers, env}`, and leave
//!   it only inside a [`ServerSpec`] for a run or a Test connection. Nothing returns them to the
//!   UI: a value field in the editor starts empty, and blank keeps the saved value.
//! - Headers go over https only, except to this Mac, so they are never sent unencrypted.
//! - A server switched off, or scoped away from an area or project, is not given to those runs,
//!   and its secrets are not read for them.

use std::{collections::BTreeMap, net::IpAddr, path::Path, time::Duration};

use serde::{Deserialize, Deserializer, Serialize};
use wack_mcp::{ServerSpec, Transport, slug};

use crate::store::{ChatKind, Store};

pub use wack_mcp::Transport as McpTransport;

pub const DEFAULT_TIMEOUT_MS: u64 = 120_000;
pub const MIN_TIMEOUT_MS: u64 = 1_000;
pub const MAX_TIMEOUT_MS: u64 = 3_600_000;
pub const MAX_SERVERS: usize = 32;
const MAX_NAME_CHARS: usize = 40;
const MAX_COMMAND_CHARS: usize = 1_000;
const MAX_ARGS: usize = 64;
const MAX_ARG_CHARS: usize = 4_000;
const MAX_ENTRIES: usize = 32;
const MAX_ENTRY_NAME_CHARS: usize = 128;
const MAX_VALUE_CHARS: usize = 8_000;
const MAX_DISABLED_TOOLS: usize = 500;
const MAX_TOOL_NAME_CHARS: usize = 200;
const MAX_TOOL_DESCRIPTION_CHARS: usize = 1_000;
/// Headers the transports set themselves; a user value would break the connection.
const RESERVED_HEADERS: &[&str] =
    &["host", "content-length", "content-type", "accept", "connection", "transfer-encoding", "mcp-session-id", "mcp-protocol-version", "last-event-id"];

/// Which Code projects a server reaches when Code is in scope.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Projects {
    #[default]
    All,
    /// Only these projects, by id.
    Only(Vec<String>),
    /// Every project except these, by id.
    Except(Vec<String>),
}

impl Projects {
    pub fn is_all(&self) -> bool {
        matches!(self, Projects::All)
    }

    pub fn filter(&self) -> ProjectFilter {
        match self {
            Projects::All => ProjectFilter::All,
            Projects::Only(_) => ProjectFilter::Only,
            Projects::Except(_) => ProjectFilter::Except,
        }
    }

    pub fn ids(&self) -> &[String] {
        match self {
            Projects::All => &[],
            Projects::Only(ids) | Projects::Except(ids) => ids,
        }
    }

    pub fn includes(&self, project: Option<&str>) -> bool {
        match self {
            Projects::All => true,
            Projects::Only(ids) => project.is_some_and(|project| ids.iter().any(|id| id == project)),
            Projects::Except(ids) => project.is_none_or(|project| !ids.iter().any(|id| id == project)),
        }
    }
}

/// Which areas a server reaches, for UI controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AreaScope {
    /// Both Code projects and the Chat area.
    #[default]
    All,
    /// Code area only (never Chat mode).
    Code,
    /// Chat area only (never Code projects).
    Chat,
}

/// Which project filter mode is active for Code, for UI controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ProjectFilter {
    #[default]
    All,
    Only,
    Except,
}

/// Where an MCP server is offered: which areas and, when Code is included, which projects.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "area", rename_all = "lowercase")]
pub enum McpScope {
    /// Both Code and Chat; `projects` filters which Code projects receive it.
    All {
        #[serde(default, skip_serializing_if = "Projects::is_all")]
        projects: Projects,
    },
    /// Code area only; `projects` filters which Code projects receive it.
    Code {
        #[serde(default, skip_serializing_if = "Projects::is_all")]
        projects: Projects,
    },
    /// Chat area only; never offered to Code projects.
    Chat,
}

impl Default for McpScope {
    fn default() -> Self {
        Self::All { projects: Projects::All }
    }
}

impl McpScope {
    pub fn area(&self) -> AreaScope {
        match self {
            McpScope::All { .. } => AreaScope::All,
            McpScope::Code { .. } => AreaScope::Code,
            McpScope::Chat => AreaScope::Chat,
        }
    }

    pub fn projects(&self) -> Option<&Projects> {
        match self {
            McpScope::All { projects } | McpScope::Code { projects } => Some(projects),
            McpScope::Chat => None,
        }
    }

    pub fn includes_chat(&self) -> bool {
        matches!(self, McpScope::All { .. } | McpScope::Chat)
    }

    pub fn includes_code(&self, project: Option<&str>) -> bool {
        match self {
            McpScope::Chat => false,
            McpScope::All { projects } | McpScope::Code { projects } => projects.includes(project),
        }
    }

    pub fn includes(&self, kind: ChatKind, project: Option<&str>) -> bool {
        match kind {
            ChatKind::Chat => self.includes_chat(),
            ChatKind::Code => self.includes_code(project),
        }
    }

    /// Switches the area while preserving the project filter between Everywhere and Code only.
    pub fn with_area(&self, area: AreaScope) -> Self {
        let projects = self.projects().cloned().unwrap_or(Projects::All);
        match area {
            AreaScope::All => McpScope::All { projects },
            AreaScope::Code => McpScope::Code { projects },
            AreaScope::Chat => McpScope::Chat,
        }
    }

    /// Switches the Code project filter, preserving any chosen project ids when flipping between
    /// `Only` and `Except`.
    pub fn with_project_filter(&self, filter: ProjectFilter) -> Self {
        let next = |current: &Projects| match filter {
            ProjectFilter::All => Projects::All,
            ProjectFilter::Only => Projects::Only(current.ids().to_vec()),
            ProjectFilter::Except => Projects::Except(current.ids().to_vec()),
        };
        match self {
            McpScope::All { projects } => McpScope::All { projects: next(projects) },
            McpScope::Code { projects } => McpScope::Code { projects: next(projects) },
            McpScope::Chat => McpScope::Chat,
        }
    }

    /// Adds or removes `project_id` from an `Only` or `Except` list.
    pub fn toggle_project(&self, project_id: &str) -> Self {
        let toggle = |projects: &Projects| match projects {
            Projects::All => Projects::All,
            Projects::Only(ids) => {
                let mut next = ids.clone();
                match next.iter().position(|id| id == project_id) {
                    Some(index) => {
                        next.remove(index);
                    }
                    None => next.push(project_id.to_string()),
                }
                Projects::Only(next)
            }
            Projects::Except(ids) => {
                let mut next = ids.clone();
                match next.iter().position(|id| id == project_id) {
                    Some(index) => {
                        next.remove(index);
                    }
                    None => next.push(project_id.to_string()),
                }
                Projects::Except(next)
            }
        };
        match self {
            McpScope::All { projects } => McpScope::All { projects: toggle(projects) },
            McpScope::Code { projects } => McpScope::Code { projects: toggle(projects) },
            McpScope::Chat => McpScope::Chat,
        }
    }

    /// Whether the scope is an `Only` or `Except` filter with no projects chosen yet.
    pub fn needs_project_choice(&self) -> bool {
        matches!(self.projects(), Some(Projects::Only(ids) | Projects::Except(ids)) if ids.is_empty())
    }

    /// Removes a deleted project from any `Only` or `Except` list. An emptied `Except` normalises
    /// to `All`; an emptied `Only` under `All` becomes `Chat`, while under `Code` it stays empty
    /// so removing a server's only allowed project never enables it everywhere.
    pub fn prune_project(&mut self, project_id: &str) -> bool {
        match self {
            McpScope::Chat => false,
            McpScope::All { projects } => match projects {
                Projects::All => false,
                Projects::Except(ids) => {
                    let before = ids.len();
                    ids.retain(|id| id != project_id);
                    if ids.len() == before {
                        return false;
                    }
                    if ids.is_empty() {
                        *projects = Projects::All;
                    }
                    true
                }
                Projects::Only(ids) => {
                    let before = ids.len();
                    ids.retain(|id| id != project_id);
                    if ids.len() == before {
                        return false;
                    }
                    if ids.is_empty() {
                        *self = McpScope::Chat;
                    }
                    true
                }
            },
            McpScope::Code { projects } => match projects {
                Projects::All => false,
                Projects::Except(ids) => {
                    let before = ids.len();
                    ids.retain(|id| id != project_id);
                    if ids.len() == before {
                        return false;
                    }
                    if ids.is_empty() {
                        *projects = Projects::All;
                    }
                    true
                }
                Projects::Only(ids) => {
                    let before = ids.len();
                    ids.retain(|id| id != project_id);
                    ids.len() != before
                }
            },
        }
    }

    /// A short human description of where this server runs, shared by Settings and `wack mcp list`.
    pub fn describe<'a>(&self, project_name: impl Fn(&str) -> Option<&'a str>) -> String {
        let list = |ids: &[String]| -> Vec<&'a str> { ids.iter().filter_map(|id| project_name(id)).collect() };
        match self {
            McpScope::Chat => "Chat only".into(),
            McpScope::All { projects: Projects::All } => "Everywhere".into(),
            McpScope::All { projects: Projects::Only(ids) } => {
                let chosen = list(ids);
                if chosen.is_empty() { "Chat only".into() } else { format!("Chat, {}", chosen.join(", ")) }
            }
            McpScope::All { projects: Projects::Except(ids) } => {
                let excluded = list(ids);
                if excluded.is_empty() { "Everywhere".into() } else { format!("Everywhere except {}", excluded.join(", ")) }
            }
            McpScope::Code { projects: Projects::All } => "Code only".into(),
            McpScope::Code { projects: Projects::Only(ids) } => {
                let chosen = list(ids);
                if chosen.is_empty() { "No projects".into() } else { format!("Only {}", chosen.join(", ")) }
            }
            McpScope::Code { projects: Projects::Except(ids) } => {
                let excluded = list(ids);
                if excluded.is_empty() { "Code only".into() } else { format!("Code except {}", excluded.join(", ")) }
            }
        }
    }
}

/// One tool as Settings lists it, from the last Test connection or listing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpToolInfo {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub read_only: bool,
}

/// A server as `state.json` keeps it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct McpServer {
    pub id: String,
    pub name: String,
    #[serde(default = "on")]
    pub enabled: bool,
    pub transport: Transport,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// Header names; the values are in `secrets.json`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<String>,
    /// Environment variable names; the values are in `secrets.json`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
    /// The server's own names for the tools the user switched off.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_tools: Vec<String>,
    /// The tools found by the last Test connection, for Settings to list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<McpToolInfo>,
    #[serde(default)]
    pub scope: McpScope,
}

/// Reads both the current `scope` field and earlier v3 files that saved `projects` alone,
/// keeping `projects: { only: […] }` restricted to Code so it never leaks into Chat.
impl<'de> Deserialize<'de> for McpServer {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct StoredMcpServer {
            id: String,
            name: String,
            #[serde(default = "on")]
            enabled: bool,
            transport: Transport,
            #[serde(default = "default_timeout")]
            timeout_ms: u64,
            #[serde(default)]
            command: String,
            #[serde(default)]
            args: Vec<String>,
            #[serde(default)]
            url: String,
            #[serde(default)]
            headers: Vec<String>,
            #[serde(default)]
            env: Vec<String>,
            #[serde(default)]
            disabled_tools: Vec<String>,
            #[serde(default)]
            tools: Vec<McpToolInfo>,
            #[serde(default)]
            scope: Option<McpScope>,
            #[serde(default)]
            projects: Option<Projects>,
        }
        let raw = StoredMcpServer::deserialize(deserializer)?;
        let scope = match (raw.scope, raw.projects) {
            (Some(scope), _) => scope,
            (None, Some(Projects::Only(ids))) => McpScope::Code { projects: Projects::Only(ids) },
            (None, Some(Projects::Except(ids))) => McpScope::All { projects: Projects::Except(ids) },
            (None, Some(Projects::All) | None) => McpScope::default(),
        };
        Ok(Self {
            id: raw.id,
            name: raw.name,
            enabled: raw.enabled,
            transport: raw.transport,
            timeout_ms: raw.timeout_ms,
            command: raw.command,
            args: raw.args,
            url: raw.url,
            headers: raw.headers,
            env: raw.env,
            disabled_tools: raw.disabled_tools,
            tools: raw.tools,
            scope,
        })
    }
}

fn on() -> bool {
    true
}

fn default_timeout() -> u64 {
    DEFAULT_TIMEOUT_MS
}

impl McpServer {
    pub fn slug(&self) -> String {
        slug(&self.name)
    }

    /// Whether two versions of a server reach it the same way, so a tool list still applies.
    fn same_connection(&self, other: &McpServer) -> bool {
        (self.transport, &self.command, &self.args, &self.url, &self.headers, &self.env) == (other.transport, &other.command, &other.args, &other.url, &other.headers, &other.env)
    }
}

/// What `state.json` keeps for Settings › MCP servers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct McpConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub servers: Vec<McpServer>,
}

/// A server's header and environment values, as stored in `secrets.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpSecrets {
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl McpSecrets {
    fn is_empty(&self) -> bool {
        self.headers.is_empty() && self.env.is_empty()
    }
}

/// A header or variable in the editor. `value` is what was typed; blank keeps the saved one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SecretEntry {
    pub name: String,
    pub value: Option<String>,
}

/// A server as Settings › MCP servers is about to save (or test) it.
#[derive(Clone, Debug, PartialEq)]
pub struct McpDraft {
    /// The server being edited; `None` for a new one.
    pub id: Option<String>,
    pub name: String,
    pub transport: Transport,
    pub timeout_ms: u64,
    pub command: String,
    pub args: Vec<String>,
    pub url: String,
    pub headers: Vec<SecretEntry>,
    pub env: Vec<SecretEntry>,
    pub scope: McpScope,
}

impl McpDraft {
    pub fn new(transport: Transport) -> Self {
        Self {
            id: None,
            name: String::new(),
            transport,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            command: String::new(),
            args: Vec::new(),
            url: String::new(),
            headers: Vec::new(),
            env: Vec::new(),
            scope: McpScope::default(),
        }
    }

    /// A saved server, ready to edit: names listed, values left blank.
    pub fn of(server: &McpServer) -> Self {
        let entries = |names: &[String]| names.iter().map(|name| SecretEntry { name: name.clone(), value: None }).collect();
        Self {
            id: Some(server.id.clone()),
            name: server.name.clone(),
            transport: server.transport,
            timeout_ms: server.timeout_ms,
            command: server.command.clone(),
            args: server.args.clone(),
            url: server.url.clone(),
            headers: entries(&server.headers),
            env: entries(&server.env),
            scope: server.scope.clone(),
        }
    }
}

pub fn secret_key(server_id: &str) -> String {
    format!("mcp:{server_id}")
}

/// Validates and normalises a server's availability scope against the projects currently saved.
pub fn validate_scope(scope: &McpScope, project_ids: &[String]) -> Result<McpScope, String> {
    let clean = |ids: &[String]| -> Vec<String> {
        let mut kept = Vec::new();
        for id in ids {
            if project_ids.contains(id) && !kept.contains(id) {
                kept.push(id.clone());
            }
        }
        kept
    };
    let check_projects = |projects: &Projects, empty_only_message: &str| -> Result<Projects, String> {
        match projects {
            Projects::All => Ok(Projects::All),
            Projects::Only(ids) => {
                let kept = clean(ids);
                if kept.is_empty() {
                    return Err(empty_only_message.into());
                }
                Ok(Projects::Only(kept))
            }
            Projects::Except(ids) => {
                let kept = clean(ids);
                if kept.is_empty() {
                    return Err("Choose at least one project to exclude, or switch to All projects.".into());
                }
                Ok(Projects::Except(kept))
            }
        }
    };
    match scope {
        McpScope::Chat => Ok(McpScope::Chat),
        McpScope::All { projects } => Ok(McpScope::All { projects: check_projects(projects, "Choose at least one project, or switch to Chat only.")? }),
        McpScope::Code { projects } => Ok(McpScope::Code { projects: check_projects(projects, "Choose at least one project, or switch to All projects.")? }),
    }
}

/// Checks an edited server against the others and resolves its secrets: a blank value keeps the
/// saved one, and a name no longer listed drops its value. Returns the record to store (keeping
/// the server's switch, tool switches and last tool list) and its secrets.
pub fn validate(draft: &McpDraft, servers: &[McpServer], saved: &McpSecrets, project_ids: &[String]) -> Result<(McpServer, McpSecrets), String> {
    let existing = match draft.id.as_deref() {
        Some(id) => Some(servers.iter().find(|server| server.id == id).ok_or("That MCP server no longer exists.")?),
        None if servers.len() >= MAX_SERVERS => return Err(format!("WackCode supports up to {MAX_SERVERS} MCP servers.")),
        None => None,
    };
    let name = draft.name.trim().to_string();
    if name.is_empty() {
        return Err("Give the server a name.".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("Keep the name to {MAX_NAME_CHARS} characters."));
    }
    if name.chars().any(char::is_control) {
        return Err("The name can't contain line breaks or control characters.".into());
    }
    let server_slug = slug(&name);
    if let Some(other) = servers.iter().find(|server| Some(server.id.as_str()) != draft.id.as_deref() && slug(&server.name) == server_slug) {
        return Err(format!("“{name}” is too close to the name of “{}”. Choose a different name.", other.name));
    }
    if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&draft.timeout_ms) {
        return Err(format!("Set a timeout between {} second and {} hour.", MIN_TIMEOUT_MS / 1_000, MAX_TIMEOUT_MS / 3_600_000));
    }
    let scope = validate_scope(&draft.scope, project_ids)?;

    let mut secrets = McpSecrets::default();
    let (command, args, url) = match draft.transport {
        Transport::Stdio => {
            let command = draft.command.trim().to_string();
            if command.is_empty() {
                return Err("Enter the command that starts the server.".into());
            }
            if command.chars().count() > MAX_COMMAND_CHARS || command.chars().any(char::is_control) {
                return Err("The command must be a single line.".into());
            }
            if draft.args.len() > MAX_ARGS {
                return Err(format!("A server can have up to {MAX_ARGS} arguments."));
            }
            if draft.args.iter().any(|arg| arg.contains('\0') || arg.chars().count() > MAX_ARG_CHARS) {
                return Err(format!("Each argument must be under {MAX_ARG_CHARS} characters, without control characters."));
            }
            secrets.env = resolve_entries(&draft.env, &saved.env, "environment variable", valid_env_name)?;
            (command, draft.args.clone(), String::new())
        }
        Transport::Http | Transport::Sse => {
            let url = validate_url(&draft.url)?;
            secrets.headers = resolve_entries(&draft.headers, &saved.headers, "header", valid_header_name)?;
            if let Some(reserved) = secrets.headers.keys().find(|name| RESERVED_HEADERS.contains(&name.to_ascii_lowercase().as_str())) {
                return Err(format!("WackCode sets the {reserved} header itself. Remove it."));
            }
            if secrets.headers.values().any(|value| value.contains(['\r', '\n'])) {
                return Err("A header value must be a single line.".into());
            }
            if !secrets.headers.is_empty() && url.starts_with("http://") && !is_loopback(&url) {
                return Err("Use an https:// URL to send headers to a remote server, so they aren't sent unencrypted.".into());
            }
            (String::new(), Vec::new(), url)
        }
    };

    let record = McpServer {
        id: existing.map_or_else(|| crate::store::new_id("mcp"), |server| server.id.clone()),
        name,
        enabled: existing.is_none_or(|server| server.enabled),
        transport: draft.transport,
        timeout_ms: draft.timeout_ms,
        command,
        args,
        url,
        headers: secrets.headers.keys().cloned().collect(),
        env: secrets.env.keys().cloned().collect(),
        disabled_tools: existing.map(|server| server.disabled_tools.clone()).unwrap_or_default(),
        tools: Vec::new(),
        scope,
    };
    // A tool list only applies while the server is reached the same way.
    let tools = existing.filter(|server| server.same_connection(&record)).map(|server| server.tools.clone()).unwrap_or_default();
    Ok((McpServer { tools, ..record }, secrets))
}

/// Header or environment entries by name, each with its value: the one typed, or else the one
/// already saved under that name.
fn resolve_entries(entries: &[SecretEntry], saved: &BTreeMap<String, String>, label: &str, valid_name: fn(&str) -> bool) -> Result<BTreeMap<String, String>, String> {
    if entries.len() > MAX_ENTRIES {
        return Err(format!("A server can have up to {MAX_ENTRIES} of these."));
    }
    let mut resolved = BTreeMap::new();
    for entry in entries {
        let name = entry.name.trim();
        if name.is_empty() && entry.value.as_deref().is_none_or(|value| value.trim().is_empty()) {
            continue;
        }
        if name.chars().count() > MAX_ENTRY_NAME_CHARS || !valid_name(name) {
            return Err(format!("“{name}” isn't a valid {label} name."));
        }
        if resolved.keys().any(|existing: &String| existing.eq_ignore_ascii_case(name)) {
            return Err(format!("The {label} {name} is listed twice."));
        }
        let typed = entry.value.as_deref().map(str::trim).filter(|value| !value.is_empty());
        let value = match typed {
            Some(value) => value.to_string(),
            None => saved.get(name).cloned().ok_or_else(|| format!("Enter a value for the {label} {name}."))?,
        };
        if value.chars().count() > MAX_VALUE_CHARS || value.contains('\0') {
            return Err(format!("The value of {name} is too long or contains a control character."));
        }
        resolved.insert(name.to_string(), value);
    }
    Ok(resolved)
}

fn valid_env_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters.next().is_some_and(|first| first.is_ascii_alphabetic() || first == '_') && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

/// An HTTP token (RFC 9110 §5.6.2).
fn valid_header_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|character| character.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(character))
}

fn validate_url(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("Enter the server's URL.".into());
    }
    let parsed = url::Url::parse(value).map_err(|_| "Enter a valid URL, such as https://example.com/mcp.".to_string())?;
    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return Err("The URL must start with https:// or http://.".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("Put credentials in a header, not in the URL.".into());
    }
    Ok(value.to_string())
}

fn is_loopback(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else { return false };
    let Some(host) = parsed.host_str() else { return false };
    let host = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    host == "localhost" || host.ends_with(".localhost") || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// The user's per-tool switches: trimmed, deduplicated and sorted.
fn validate_disabled_tools(names: &[String]) -> Result<Vec<String>, String> {
    let mut clean: Vec<String> = names.iter().map(|name| name.trim().to_string()).filter(|name| !name.is_empty()).collect();
    clean.sort();
    clean.dedup();
    if clean.len() > MAX_DISABLED_TOOLS || clean.iter().any(|name| name.chars().count() > MAX_TOOL_NAME_CHARS) {
        return Err("That list of tools is too long.".into());
    }
    Ok(clean)
}

/// One server as a connection needs it, with its secrets.
pub fn spec(server: &McpServer, secrets: &McpSecrets) -> ServerSpec {
    let stdio = server.transport == Transport::Stdio;
    ServerSpec {
        id: server.id.clone(),
        name: server.name.clone(),
        slug: server.slug(),
        transport: server.transport,
        timeout: Duration::from_millis(server.timeout_ms),
        command: server.command.clone(),
        args: server.args.clone(),
        env: if stdio { secrets.env.clone() } else { BTreeMap::new() },
        url: server.url.clone(),
        headers: if stdio { BTreeMap::new() } else { secrets.headers.clone() },
        disabled_tools: server.disabled_tools.clone(),
    }
}

impl Store {
    pub fn mcp_servers(&self) -> &[McpServer] {
        &self.mcp_state().servers
    }

    pub fn mcp_server(&self, id: &str) -> Option<&McpServer> {
        self.mcp_servers().iter().find(|server| server.id == id)
    }

    fn mcp_secrets(&self, id: &str) -> McpSecrets {
        self.secret(&secret_key(id)).and_then(|json| serde_json::from_str(json).ok()).unwrap_or_default()
    }

    /// The names of a saved server's values, for the editor to say "saved" beside them.
    pub fn mcp_saved_names(&self, id: &str) -> Vec<String> {
        let secrets = self.mcp_secrets(id);
        secrets.headers.into_keys().chain(secrets.env.into_keys()).collect()
    }

    /// Saves a server and its values. Returns its id.
    pub fn save_mcp_server(&mut self, draft: &McpDraft) -> Result<String, String> {
        let saved = draft.id.as_deref().map(|id| self.mcp_secrets(id)).unwrap_or_default();
        let project_ids: Vec<String> = self.projects().iter().map(|project| project.id.clone()).collect();
        let (record, secrets) = validate(draft, self.mcp_servers(), &saved, &project_ids)?;
        if secrets.is_empty() {
            self.remove_secret(&secret_key(&record.id))?;
        } else {
            let json = serde_json::to_string(&secrets).map_err(|_| "Could not save the server's values.".to_string())?;
            self.set_secret(&secret_key(&record.id), &json)?;
        }
        let id = record.id.clone();
        let servers = &mut self.mcp_state_mut().servers;
        match servers.iter_mut().find(|server| server.id == id) {
            Some(saved) => *saved = record,
            None => servers.push(record),
        }
        self.save_state()?;
        Ok(id)
    }

    pub fn delete_mcp_server(&mut self, id: &str) -> Result<(), String> {
        self.mcp_state_mut().servers.retain(|server| server.id != id);
        self.save_state()?;
        self.remove_secret(&secret_key(id))
    }

    pub fn set_mcp_server_enabled(&mut self, id: &str, enabled: bool) -> Result<(), String> {
        let server = self.mcp_state_mut().servers.iter_mut().find(|server| server.id == id).ok_or("That MCP server no longer exists.")?;
        if server.enabled == enabled {
            return Ok(());
        }
        server.enabled = enabled;
        self.save_state()
    }

    /// Updates where a saved server is available (area and project filter) without touching its
    /// connection, secrets, or known tools.
    pub fn set_mcp_server_scope(&mut self, id: &str, scope: McpScope) -> Result<(), String> {
        let project_ids: Vec<String> = self.projects().iter().map(|project| project.id.clone()).collect();
        let scope = validate_scope(&scope, &project_ids)?;
        let server = self.mcp_state_mut().servers.iter_mut().find(|server| server.id == id).ok_or("That MCP server no longer exists.")?;
        if server.scope == scope {
            return Ok(());
        }
        server.scope = scope;
        self.save_state()
    }

    /// Removes a deleted project from every server's scope lists.
    pub(crate) fn prune_mcp_project(&mut self, project_id: &str) {
        for server in &mut self.mcp_state_mut().servers {
            server.scope.prune_project(project_id);
        }
    }

    /// Switches one of a server's tools on or off, by the server's own name for it.
    pub fn set_mcp_tool_enabled(&mut self, id: &str, tool: &str, enabled: bool) -> Result<(), String> {
        let server = self.mcp_state_mut().servers.iter_mut().find(|server| server.id == id).ok_or("That MCP server no longer exists.")?;
        let mut disabled = server.disabled_tools.clone();
        disabled.retain(|name| name != tool);
        if !enabled {
            disabled.push(tool.to_string());
        }
        server.disabled_tools = validate_disabled_tools(&disabled)?;
        self.save_state()
    }

    /// Keeps the tools a Test connection found, for Settings to list. Ignored if the server was
    /// changed or removed meanwhile.
    pub fn set_mcp_tools(&mut self, id: &str, fingerprint: &str, tools: &[wack_mcp::ToolDecl]) -> Result<(), String> {
        let Some(server) = self.mcp_server(id) else { return Ok(()) };
        if spec(server, &self.mcp_secrets(id)).fingerprint() != fingerprint {
            return Ok(());
        }
        let infos: Vec<McpToolInfo> = tools
            .iter()
            .take(wack_mcp::MAX_TOOLS)
            .map(|tool| McpToolInfo {
                name: tool.name.chars().take(MAX_TOOL_NAME_CHARS).collect(),
                description: tool.description.as_deref().unwrap_or_default().chars().take(MAX_TOOL_DESCRIPTION_CHARS).collect(),
                read_only: tool.read_only(),
            })
            .collect();
        let server = self.mcp_state_mut().servers.iter_mut().find(|server| server.id == id).expect("found above");
        if server.tools == infos {
            return Ok(());
        }
        server.tools = infos;
        self.save_state()
    }

    /// The servers a run gets: switched on and whose scope includes `kind` and (for Code) the
    /// project at `folder`. A Code run in an unregistered folder (`wack run` elsewhere) gets
    /// `All` and `Except` servers, never `Only` or `Chat` servers.
    pub fn mcp_specs_for(&self, kind: ChatKind, folder: &Path) -> Vec<ServerSpec> {
        let project = match kind {
            ChatKind::Code => self.projects().iter().find(|project| project.path == folder).map(|project| project.id.as_str()),
            ChatKind::Chat => None,
        };
        self.mcp_servers()
            .iter()
            .filter(|server| server.enabled && server.scope.includes(kind, project))
            .map(|server| spec(server, &self.mcp_secrets(&server.id)))
            .collect()
    }

    /// A saved server ready to connect, switched on or not (Test connection, `wack mcp test`).
    pub fn mcp_spec(&self, id: &str) -> Option<ServerSpec> {
        self.mcp_server(id).map(|server| spec(server, &self.mcp_secrets(id)))
    }

    /// A draft ready to connect, for Test connection before saving. Blank values use the saved
    /// ones.
    pub fn mcp_draft_spec(&self, draft: &McpDraft) -> Result<ServerSpec, String> {
        let saved = draft.id.as_deref().map(|id| self.mcp_secrets(id)).unwrap_or_default();
        let project_ids: Vec<String> = self.projects().iter().map(|project| project.id.clone()).collect();
        let (record, secrets) = validate(draft, self.mcp_servers(), &saved, &project_ids)?;
        Ok(spec(&record, &secrets))
    }

    /// Whether a live hub connection `(id, fingerprint, folder)` is still enabled, matches the
    /// saved connection configuration, and remains in scope for `folder`.
    pub fn mcp_keeps_connection(&self, id: &str, fingerprint: &str, folder: &Path) -> bool {
        let Some(server) = self.mcp_server(id).filter(|server| server.enabled) else {
            return false;
        };
        if spec(server, &self.mcp_secrets(id)).fingerprint() != fingerprint {
            return false;
        }
        if folder == self.chat_area_folder_path() {
            return server.scope.includes_chat();
        }
        let Some(project) = self.projects().iter().find(|project| project.path == folder) else {
            return false;
        };
        server.scope.includes_code(Some(&project.id))
    }

    /// Where each server's last tool list is cached.
    pub fn mcp_cache_folder(&self) -> std::path::PathBuf {
        self.folder().join("mcp-cache")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, value: Option<&str>) -> SecretEntry {
        SecretEntry { name: name.into(), value: value.map(str::to_string) }
    }

    fn stdio(name: &str) -> McpDraft {
        McpDraft {
            name: name.into(),
            command: "npx".into(),
            args: vec!["-y".into(), "@example/server".into()],
            env: vec![entry("GITHUB_TOKEN", Some("ghp_secretvalue123"))],
            ..McpDraft::new(Transport::Stdio)
        }
    }

    fn http(url: &str, headers: Vec<SecretEntry>) -> McpDraft {
        McpDraft { name: "Remote".into(), url: url.into(), headers, ..McpDraft::new(Transport::Http) }
    }

    fn check(draft: &McpDraft) -> Result<(McpServer, McpSecrets), String> {
        validate(draft, &[], &McpSecrets::default(), &[])
    }

    #[test]
    fn a_new_stdio_server_keeps_values_out_of_the_record() {
        let (record, secrets) = check(&stdio("GitHub")).unwrap();
        assert!(record.id.starts_with("mcp"));
        assert!(record.enabled);
        assert_eq!(record.env, ["GITHUB_TOKEN"]);
        assert_eq!(secrets.env["GITHUB_TOKEN"], "ghp_secretvalue123");
        assert!(!serde_json::to_string(&record).unwrap().contains("ghp_secretvalue123"));
    }

    #[test]
    fn a_blank_value_keeps_the_saved_one_and_a_missing_one_is_refused() {
        let (record, secrets) = check(&stdio("GitHub")).unwrap();
        let mut edit = stdio("GitHub");
        edit.id = Some(record.id.clone());
        edit.env = vec![entry("GITHUB_TOKEN", Some("  ")), entry("OTHER", None)];
        let error = validate(&edit, std::slice::from_ref(&record), &secrets, &[]).unwrap_err();
        assert!(error.contains("OTHER"), "{error}");
        edit.env = vec![entry("GITHUB_TOKEN", None)];
        let (_, kept) = validate(&edit, std::slice::from_ref(&record), &secrets, &[]).unwrap();
        assert_eq!(kept.env["GITHUB_TOKEN"], "ghp_secretvalue123");
    }

    #[test]
    fn editing_keeps_the_switch_and_tool_state_while_the_connection_stays() {
        let (mut record, secrets) = check(&stdio("GitHub")).unwrap();
        record.enabled = false;
        record.disabled_tools = vec!["delete_repo".into()];
        record.tools = vec![McpToolInfo { name: "delete_repo".into(), description: String::new(), read_only: false }];
        let mut edit = stdio("GitHub renamed");
        edit.id = Some(record.id.clone());
        let (saved, _) = validate(&edit, std::slice::from_ref(&record), &secrets, &[]).unwrap();
        assert!(!saved.enabled);
        assert_eq!(saved.disabled_tools, record.disabled_tools);
        assert_eq!(saved.tools, record.tools);
        assert_eq!(saved.name, "GitHub renamed");
        edit.command = "uvx".into();
        let (moved, _) = validate(&edit, std::slice::from_ref(&record), &secrets, &[]).unwrap();
        assert!(moved.tools.is_empty());
    }

    #[test]
    fn names_must_be_distinct_once_slugged() {
        let (record, _) = check(&stdio("GitHub")).unwrap();
        assert!(validate(&stdio("github!"), std::slice::from_ref(&record), &McpSecrets::default(), &[]).is_err());
        assert!(check(&stdio("  ")).is_err());
    }

    #[test]
    fn timeouts_commands_and_variables_are_checked() {
        let mut draft = stdio("A");
        draft.timeout_ms = 500;
        assert!(check(&draft).is_err());
        let mut draft = stdio("A");
        draft.command = "npx\nrm -rf".into();
        assert!(check(&draft).is_err());
        let mut draft = stdio("A");
        draft.env = vec![entry("1BAD", Some("x"))];
        assert!(check(&draft).is_err());
    }

    #[test]
    fn headers_need_https_unless_the_server_is_local() {
        let bearer = || vec![entry("Authorization", Some("Bearer abcdefgh12345"))];
        assert!(check(&http("http://mcp.example.com/mcp", bearer())).is_err());
        assert!(check(&http("https://mcp.example.com/mcp", bearer())).is_ok());
        assert!(check(&http("http://127.0.0.1:8080/mcp", bearer())).is_ok());
        assert!(check(&http("http://localhost:8080/mcp", bearer())).is_ok());
        assert!(check(&http("http://[::1]:8080/mcp", bearer())).is_ok());
        assert!(check(&http("http://mcp.example.com/mcp", Vec::new())).is_ok());
    }

    #[test]
    fn urls_and_header_names_are_checked() {
        assert!(check(&http("ftp://example.com", Vec::new())).is_err());
        assert!(check(&http("https://user:pw@example.com/mcp", Vec::new())).is_err());
        assert!(check(&http("https://example.com", vec![entry("Bad Header", Some("x"))])).is_err());
        assert!(check(&http("https://example.com", vec![entry("Content-Type", Some("x"))])).is_err());
        assert!(check(&http("https://example.com", vec![entry("X-Key", Some("a\nb"))])).is_err());
        assert!(check(&http("https://example.com", vec![entry("X-Key", Some("a")), entry("x-key", Some("b"))])).is_err());
    }

    #[test]
    fn scopes_validate_deduplicate_and_filter_areas_and_projects() {
        let mut draft = stdio("A");
        draft.scope = McpScope::Code { projects: Projects::Only(vec!["gone".into()]) };
        assert!(check(&draft).unwrap_err().contains("at least one project"));

        draft.scope = McpScope::All { projects: Projects::Only(vec!["gone".into()]) };
        assert!(check(&draft).unwrap_err().contains("Chat only"));

        draft.scope = McpScope::Code { projects: Projects::Except(vec!["gone".into()]) };
        assert!(check(&draft).unwrap_err().contains("exclude"));

        draft.scope = McpScope::Code { projects: Projects::Only(vec!["p1".into(), "gone".into(), "p1".into()]) };
        let (record, _) = validate(&draft, &[], &McpSecrets::default(), &["p1".into(), "p2".into()]).unwrap();
        assert_eq!(record.scope, McpScope::Code { projects: Projects::Only(vec!["p1".into()]) });
        assert!(record.scope.includes(ChatKind::Code, Some("p1")));
        assert!(!record.scope.includes(ChatKind::Code, Some("p2")));
        assert!(!record.scope.includes(ChatKind::Code, None));
        assert!(!record.scope.includes(ChatKind::Chat, None));

        // Excluding all current projects is valid: future projects (and Chat for All) still get it.
        draft.scope = McpScope::All { projects: Projects::Except(vec!["p1".into(), "p2".into()]) };
        let (except_all, _) = validate(&draft, &[], &McpSecrets::default(), &["p1".into(), "p2".into()]).unwrap();
        assert!(!except_all.scope.includes(ChatKind::Code, Some("p1")));
        assert!(!except_all.scope.includes(ChatKind::Code, Some("p2")));
        assert!(except_all.scope.includes(ChatKind::Code, Some("p3")));
        assert!(except_all.scope.includes(ChatKind::Code, None));
        assert!(except_all.scope.includes(ChatKind::Chat, None));

        // Chat only never includes Code.
        draft.scope = McpScope::Chat;
        let (chat_only, _) = check(&draft).unwrap();
        assert!(chat_only.scope.includes(ChatKind::Chat, None));
        assert!(!chat_only.scope.includes(ChatKind::Code, Some("p1")));
        assert!(!chat_only.scope.includes(ChatKind::Code, None));
    }

    #[test]
    fn legacy_projects_field_migrates_without_leaking_project_only_servers_into_chat() {
        let all_json = r#"{"id":"mcp-1","name":"GitHub","transport":"stdio","command":"npx","projects":"all"}"#;
        let all: McpServer = serde_json::from_str(all_json).unwrap();
        assert_eq!(all.scope, McpScope::All { projects: Projects::All });
        assert!(all.scope.includes_chat());
        assert!(all.scope.includes_code(Some("p1")));

        let only_json = r#"{"id":"mcp-2","name":"Blender","transport":"stdio","command":"uvx","projects":{"only":["p1"]}}"#;
        let only: McpServer = serde_json::from_str(only_json).unwrap();
        assert_eq!(only.scope, McpScope::Code { projects: Projects::Only(vec!["p1".into()]) });
        assert!(!only.scope.includes_chat(), "a project-scoped server must stay out of Chat");
        assert!(only.scope.includes_code(Some("p1")));
        assert!(!only.scope.includes_code(Some("p2")));

        // Serialising writes only `scope`, never the legacy `projects` key at top level.
        let written = serde_json::to_string(&only).unwrap();
        assert!(written.contains("\"scope\""), "{written}");
        assert!(!written.contains("\"projects\":\"all\""), "{written}");
    }

    #[test]
    fn scope_helpers_preserve_selections_and_describe_every_combination() {
        let names = |id: &str| match id {
            "p1" => Some("Game"),
            "p2" => Some("App"),
            _ => None,
        };
        let s = McpScope::default();
        assert_eq!(s.describe(names), "Everywhere");
        let s = s.with_area(AreaScope::Code);
        assert_eq!(s.describe(names), "Code only");
        let s = s.with_project_filter(ProjectFilter::Only);
        assert!(s.needs_project_choice());
        assert_eq!(s.describe(names), "No projects");
        let s = s.toggle_project("p1");
        assert!(!s.needs_project_choice());
        assert_eq!(s.describe(names), "Only Game");
        let s = s.with_area(AreaScope::All);
        assert_eq!(s.describe(names), "Chat, Game");
        let s = s.with_project_filter(ProjectFilter::Except);
        assert_eq!(s.describe(names), "Everywhere except Game");
        let s = s.toggle_project("p1").toggle_project("p2");
        assert_eq!(s.describe(names), "Everywhere except App");
        let s = s.with_area(AreaScope::Code);
        assert_eq!(s.describe(names), "Code except App");
        let s = s.with_area(AreaScope::Chat);
        assert_eq!(s.describe(names), "Chat only");
    }

    #[test]
    fn runs_get_enabled_servers_in_scope_with_their_own_secrets() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        let game = store.add_project(folder.path()).unwrap().id.clone();
        let elsewhere = tempfile::tempdir().unwrap();
        let app = store.add_project(elsewhere.path()).unwrap().id.clone();
        let (game_path, app_path) = (store.project(&game).unwrap().path.clone(), store.project(&app).unwrap().path.clone());
        let chat_folder = store.chat_area_folder().unwrap();
        let adhoc = tempfile::tempdir().unwrap();

        let everywhere = store.save_mcp_server(&stdio("GitHub")).unwrap();

        let mut blender_draft = http("https://example.com/mcp", vec![entry("Authorization", Some("Bearer abcdefgh12345"))]);
        blender_draft.name = "Blender".into();
        blender_draft.scope = McpScope::Code { projects: Projects::Only(vec![game.clone()]) };
        let blender = store.save_mcp_server(&blender_draft).unwrap();

        let mut code_except_app = stdio("Linter");
        code_except_app.env.clear();
        code_except_app.scope = McpScope::Code { projects: Projects::Except(vec![app.clone()]) };
        let linter = store.save_mcp_server(&code_except_app).unwrap();

        let mut chat_only = stdio("Notes");
        chat_only.env.clear();
        chat_only.scope = McpScope::Chat;
        let notes = store.save_mcp_server(&chat_only).unwrap();

        let names = |specs: Vec<ServerSpec>| specs.into_iter().map(|spec| spec.name).collect::<Vec<_>>();
        assert_eq!(names(store.mcp_specs_for(ChatKind::Code, &game_path)), ["GitHub", "Blender", "Linter"]);
        assert_eq!(names(store.mcp_specs_for(ChatKind::Code, &app_path)), ["GitHub"]);
        assert_eq!(names(store.mcp_specs_for(ChatKind::Code, adhoc.path())), ["GitHub", "Linter"]);
        assert_eq!(names(store.mcp_specs_for(ChatKind::Chat, &chat_folder)), ["GitHub", "Notes"]);

        let specs = store.mcp_specs_for(ChatKind::Code, &game_path);
        assert_eq!(specs[0].env["GITHUB_TOKEN"], "ghp_secretvalue123");
        assert!(specs[0].headers.is_empty());
        assert_eq!(specs[1].headers["Authorization"], "Bearer abcdefgh12345");
        assert!(specs[1].env.is_empty());

        // Live connection retention honours enabled state, fingerprint and folder scope.
        let blender_fp = store.mcp_spec(&blender).unwrap().fingerprint();
        let notes_fp = store.mcp_spec(&notes).unwrap().fingerprint();
        let linter_fp = store.mcp_spec(&linter).unwrap().fingerprint();
        assert!(store.mcp_keeps_connection(&blender, &blender_fp, &game_path));
        assert!(!store.mcp_keeps_connection(&blender, &blender_fp, &app_path));
        assert!(!store.mcp_keeps_connection(&blender, &blender_fp, &chat_folder));
        assert!(store.mcp_keeps_connection(&notes, &notes_fp, &chat_folder));
        assert!(!store.mcp_keeps_connection(&notes, &notes_fp, &game_path));
        assert!(store.mcp_keeps_connection(&linter, &linter_fp, &game_path));
        assert!(!store.mcp_keeps_connection(&linter, &linter_fp, &app_path));
        assert!(!store.mcp_keeps_connection(&linter, &linter_fp, adhoc.path()));

        // Inline scope update applies immediately without touching tools or secrets.
        store.set_mcp_server_scope(&everywhere, McpScope::All { projects: Projects::Except(vec![app.clone()]) }).unwrap();
        assert_eq!(names(store.mcp_specs_for(ChatKind::Code, &app_path)), Vec::<String>::new());
        assert_eq!(names(store.mcp_specs_for(ChatKind::Chat, &chat_folder)), ["GitHub", "Notes"]);

        // Removing a project prunes Except back to All, keeps Code+Only empty (never leaks),
        // and retires its live connections.
        store.remove_project(&app).unwrap();
        assert_eq!(store.mcp_server(&linter).unwrap().scope, McpScope::Code { projects: Projects::All });
        assert_eq!(store.mcp_server(&everywhere).unwrap().scope, McpScope::All { projects: Projects::All });
        store.remove_project(&game).unwrap();
        assert_eq!(store.mcp_server(&blender).unwrap().scope, McpScope::Code { projects: Projects::Only(Vec::new()) });
        assert!(!store.mcp_keeps_connection(&blender, &blender_fp, &game_path));

        store.set_mcp_server_enabled(&everywhere, false).unwrap();
        assert_eq!(names(store.mcp_specs_for(ChatKind::Chat, &chat_folder)), ["Notes"]);
        store.set_mcp_tool_enabled(&everywhere, "delete_repo", false).unwrap();
        assert_eq!(store.mcp_server(&everywhere).unwrap().disabled_tools, ["delete_repo"]);
        store.set_mcp_tool_enabled(&everywhere, "delete_repo", true).unwrap();
        assert!(store.mcp_server(&everywhere).unwrap().disabled_tools.is_empty());

        // Values never reach state.json, and leave secrets.json with the server.
        let state = std::fs::read_to_string(folder.path().join("state.json")).unwrap();
        assert!(!state.contains("ghp_secretvalue123") && !state.contains("abcdefgh12345"));
        store.delete_mcp_server(&everywhere).unwrap();
        let secrets = std::fs::read_to_string(folder.path().join("secrets.json")).unwrap();
        assert!(!secrets.contains("ghp_secretvalue123"));
        assert!(secrets.contains("abcdefgh12345"));

        let reopened = Store::open(folder.path()).unwrap();
        assert_eq!(reopened.mcp_servers().len(), 3);
        assert_eq!(reopened.mcp_server(&notes).unwrap().scope, McpScope::Chat);
    }

    #[test]
    fn removing_the_only_project_from_all_only_keeps_chat_without_leaking_to_code() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        let project = store.add_project(folder.path()).unwrap();
        let mut draft = stdio("Mixed");
        draft.scope = McpScope::All { projects: Projects::Only(vec![project.id.clone()]) };
        let id = store.save_mcp_server(&draft).unwrap();
        store.remove_project(&project.id).unwrap();
        assert_eq!(store.mcp_server(&id).unwrap().scope, McpScope::Chat);
    }

    #[test]
    fn a_test_connections_tools_are_kept_only_for_the_same_connection() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        let project = store.add_project(folder.path()).unwrap();
        let id = store.save_mcp_server(&stdio("GitHub")).unwrap();
        let fingerprint = store.mcp_spec(&id).unwrap().fingerprint();
        let peek = wack_mcp::ToolDecl { name: "peek".into(), title: None, description: Some("Peek.".into()), input_schema: serde_json::json!({}), annotations: None };
        store.set_mcp_tools(&id, "stale", std::slice::from_ref(&peek)).unwrap();
        assert!(store.mcp_server(&id).unwrap().tools.is_empty());
        store.set_mcp_tools(&id, &fingerprint, &[peek]).unwrap();
        assert_eq!(store.mcp_server(&id).unwrap().tools[0].name, "peek");
        assert!(store.mcp_keeps_connection(&id, &fingerprint, &project.path));
        assert!(!store.mcp_keeps_connection(&id, "stale", &project.path));
    }
}
