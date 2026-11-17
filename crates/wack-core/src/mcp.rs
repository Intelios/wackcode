//! Settings › MCP servers: the servers the user added, which projects each reaches, and what a
//! run is given. The client itself is `wack_mcp`. Ported from version 2's `mcp.rs`, with its
//! validation and tests; scoping a server to projects is new.
//!
//! Invariants:
//! - `state.json` keeps each server with its header and environment variable *names* only. Their
//!   values, often tokens, live in `secrets.json` under `mcp:<id>` as `{headers, env}`, and leave
//!   it only inside a [`ServerSpec`] for a run or a Test connection. Nothing returns them to the
//!   UI: a value field in the editor starts empty, and blank keeps the saved value.
//! - Headers go over https only, except to this Mac, so they are never sent unencrypted.
//! - A server switched off, or scoped away from a project, is not given to that project's runs,
//!   and its secrets are not read for them.

use std::{collections::BTreeMap, net::IpAddr, time::Duration};

use serde::{Deserialize, Serialize};
use wack_mcp::{ServerSpec, Transport, slug};

use crate::store::Store;

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

/// Which projects' chats a server reaches.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Projects {
    #[default]
    All,
    /// These projects, by id.
    Only(Vec<String>),
}

impl Projects {
    pub fn includes(&self, project: Option<&str>) -> bool {
        match self {
            Projects::All => true,
            Projects::Only(ids) => project.is_some_and(|project| ids.iter().any(|id| id == project)),
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    pub projects: Projects,
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
    pub projects: Projects,
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
            projects: Projects::All,
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
            projects: server.projects.clone(),
        }
    }
}

pub fn secret_key(server_id: &str) -> String {
    format!("mcp:{server_id}")
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
    let projects = match &draft.projects {
        Projects::All => Projects::All,
        Projects::Only(ids) => {
            let mut kept: Vec<String> = ids.iter().filter(|id| project_ids.contains(id)).cloned().collect();
            kept.dedup();
            if kept.is_empty() {
                return Err("Choose at least one project, or All projects.".into());
            }
            Projects::Only(kept)
        }
    };

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
        projects,
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

    /// The servers a run in `folder` gets: switched on and reaching its project. A folder that
    /// is no project (a `wack run` elsewhere) gets the servers for all projects.
    pub fn mcp_specs_for(&self, folder: &std::path::Path) -> Vec<ServerSpec> {
        let project = self.projects().iter().find(|project| project.path == folder).map(|project| project.id.as_str());
        self.mcp_servers()
            .iter()
            .filter(|server| server.enabled && server.projects.includes(project))
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

    /// The connections that may stay up: each enabled server's id with its fingerprint.
    pub fn mcp_live_keys(&self) -> Vec<(String, String)> {
        self.mcp_servers().iter().filter(|server| server.enabled).map(|server| (server.id.clone(), spec(server, &self.mcp_secrets(&server.id)).fingerprint())).collect()
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
    fn projects_must_exist_and_one_must_be_chosen() {
        let mut draft = stdio("A");
        draft.projects = Projects::Only(vec!["gone".into()]);
        assert!(check(&draft).unwrap_err().contains("at least one project"));
        draft.projects = Projects::Only(vec!["p1".into(), "gone".into()]);
        let (record, _) = validate(&draft, &[], &McpSecrets::default(), &["p1".into()]).unwrap();
        assert_eq!(record.projects, Projects::Only(vec!["p1".into()]));
        assert!(record.projects.includes(Some("p1")));
        assert!(!record.projects.includes(Some("p2")));
        assert!(!record.projects.includes(None));
        assert!(Projects::All.includes(None));
    }

    #[test]
    fn runs_get_enabled_servers_in_scope_with_their_own_secrets() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        let first = store.add_project(folder.path()).unwrap().id.clone();
        let elsewhere = tempfile::tempdir().unwrap();
        let other = store.add_project(elsewhere.path()).unwrap();
        let (first_path, other_path) = (store.project(&first).unwrap().path.clone(), other.path.clone());

        let everywhere = store.save_mcp_server(&stdio("GitHub")).unwrap();
        let mut scoped = http("https://example.com/mcp", vec![entry("Authorization", Some("Bearer abcdefgh12345"))]);
        scoped.name = "Blender".into();
        scoped.projects = Projects::Only(vec![first.clone()]);
        store.save_mcp_server(&scoped).unwrap();

        let names = |specs: Vec<ServerSpec>| specs.into_iter().map(|spec| spec.name).collect::<Vec<_>>();
        assert_eq!(names(store.mcp_specs_for(&first_path)), ["GitHub", "Blender"]);
        assert_eq!(names(store.mcp_specs_for(&other_path)), ["GitHub"]);
        let specs = store.mcp_specs_for(&first_path);
        assert_eq!(specs[0].env["GITHUB_TOKEN"], "ghp_secretvalue123");
        assert!(specs[0].headers.is_empty());
        assert_eq!(specs[1].headers["Authorization"], "Bearer abcdefgh12345");
        assert!(specs[1].env.is_empty());

        store.set_mcp_server_enabled(&everywhere, false).unwrap();
        assert_eq!(names(store.mcp_specs_for(&other_path)), Vec::<String>::new());
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
        assert_eq!(reopened.mcp_servers().len(), 1);
        assert_eq!(reopened.mcp_servers()[0].projects, Projects::Only(vec![first]));
    }

    #[test]
    fn a_test_connections_tools_are_kept_only_for_the_same_connection() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        let id = store.save_mcp_server(&stdio("GitHub")).unwrap();
        let fingerprint = store.mcp_spec(&id).unwrap().fingerprint();
        let peek = wack_mcp::ToolDecl { name: "peek".into(), title: None, description: Some("Peek.".into()), input_schema: serde_json::json!({}), annotations: None };
        store.set_mcp_tools(&id, "stale", std::slice::from_ref(&peek)).unwrap();
        assert!(store.mcp_server(&id).unwrap().tools.is_empty());
        store.set_mcp_tools(&id, &fingerprint, &[peek]).unwrap();
        assert_eq!(store.mcp_server(&id).unwrap().tools[0].name, "peek");
        assert_eq!(store.mcp_live_keys(), [(id, fingerprint)]);
    }
}
