//! MCP servers: the settings behind the worker's MCP built-in (Settings › MCP servers), what a
//! worker receives, and "Test connection".
//!
//! `wackcode.json` keeps each server with its header and environment variable *names* only.
//! Their values, often tokens, live in `secrets.json` under `mcp:<id>` and reach a chat worker or
//! the test process only over its stdin, like every other credential. An environment value goes
//! further only to the stdio server it belongs to, as that process's environment.

use crate::models::{McpServerRecord, McpToolInfo, McpTransport, SaveMcpServerInput};
use crate::secrets::SecretStore;
use nix::{sys::signal::{killpg, Signal}, unistd::Pid};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, net::IpAddr, process::Stdio, time::Duration};
use tauri::{AppHandle, Manager};
use tokio::{io::AsyncWriteExt, process::Command};
use uuid::Uuid;

pub const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MIN_TIMEOUT_MS: u64 = 1_000;
const MAX_TIMEOUT_MS: u64 = 3_600_000;
const MAX_SERVERS: usize = 32;
const MAX_NAME_CHARS: usize = 40;
/// Tool names are `mcp__<slug>__<tool>` within 64 characters, so the slug stays short.
const MAX_SLUG_CHARS: usize = 20;
const MAX_COMMAND_CHARS: usize = 1_000;
const MAX_ARGS: usize = 64;
const MAX_ARG_CHARS: usize = 4_000;
const MAX_ENTRIES: usize = 32;
const MAX_ENTRY_NAME_CHARS: usize = 128;
const MAX_VALUE_CHARS: usize = 8_000;
const MAX_DISABLED_TOOLS: usize = 500;
const MAX_TOOL_NAME_CHARS: usize = 200;
const MAX_LISTED_TOOLS: usize = 500;
const MAX_TOOL_DESCRIPTION_CHARS: usize = 1_000;
/// "Test connection" never runs longer than this, whatever the server's own timeout.
const MAX_PROBE: Duration = Duration::from_secs(600);
/// Headers the MCP transports set themselves; a user value would break the connection.
const RESERVED_HEADERS: &[&str] = &[
    "host", "content-length", "content-type", "accept", "connection", "transfer-encoding",
    "mcp-session-id", "mcp-protocol-version", "last-event-id",
];

/// A server's header and environment values, as stored in `secrets.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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

pub fn secret_key(server_id: &str) -> String {
    format!("mcp:{server_id}")
}

pub fn load_secrets(store: &SecretStore, server_id: &str) -> McpSecrets {
    store.get_optional(&secret_key(server_id)).ok().flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

pub fn store_secrets(store: &SecretStore, server_id: &str, secrets: &McpSecrets) -> Result<(), String> {
    if secrets.is_empty() { return store.remove(&secret_key(server_id)); }
    store.set(&secret_key(server_id), &serde_json::to_string(secrets).map_err(|error| error.to_string())?)
}

/// The middle of the server's tool names: lowercase letters, digits and underscores.
pub fn slug(name: &str) -> String {
    let raw: String = name.chars().map(|character| {
        if character.is_ascii_alphanumeric() { character.to_ascii_lowercase() } else { '_' }
    }).collect();
    let joined = raw.split('_').filter(|part| !part.is_empty()).collect::<Vec<_>>().join("_");
    let short: String = joined.chars().take(MAX_SLUG_CHARS).collect();
    let short = short.trim_end_matches('_');
    if short.is_empty() { "server".into() } else { short.to_string() }
}

/// Check an edited server against the others and resolve its secrets: a blank value keeps the
/// saved one, and a name no longer listed drops its value. Returns the record to store (keeping
/// the server's switch, per-tool switches and last tool list) and its secrets.
pub fn validate(input: &SaveMcpServerInput, servers: &[McpServerRecord], saved: &McpSecrets) -> Result<(McpServerRecord, McpSecrets), String> {
    let existing = match input.id.as_deref() {
        Some(id) => Some(servers.iter().find(|server| server.id == id).ok_or("That MCP server no longer exists.")?),
        None if servers.len() >= MAX_SERVERS => return Err(format!("WackCode supports up to {MAX_SERVERS} MCP servers.")),
        None => None,
    };
    let name = input.name.trim().to_string();
    if name.is_empty() { return Err("Give the server a name.".into()); }
    if name.chars().count() > MAX_NAME_CHARS { return Err(format!("Keep the name to {MAX_NAME_CHARS} characters.")); }
    if name.chars().any(char::is_control) { return Err("The name can't contain line breaks or control characters.".into()); }
    let server_slug = slug(&name);
    if let Some(other) = servers.iter().find(|server| Some(server.id.as_str()) != input.id.as_deref() && slug(&server.name) == server_slug) {
        return Err(format!("\u{201c}{name}\u{201d} is too close to the name of \u{201c}{}\u{201d}. Choose a different name.", other.name));
    }
    if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&input.timeout_ms) {
        return Err(format!("Set a timeout between {MIN_TIMEOUT_MS} and {MAX_TIMEOUT_MS} ms."));
    }

    let mut secrets = McpSecrets::default();
    let (command, args, url, headers, env) = match input.transport {
        McpTransport::Stdio => {
            let command = input.command.trim().to_string();
            if command.is_empty() { return Err("Enter the command that starts the server.".into()); }
            if command.chars().count() > MAX_COMMAND_CHARS || command.chars().any(char::is_control) {
                return Err("The command must be a single line.".into());
            }
            if input.args.len() > MAX_ARGS { return Err(format!("A server can have up to {MAX_ARGS} arguments.")); }
            if input.args.iter().any(|arg| arg.contains('\0') || arg.chars().count() > MAX_ARG_CHARS) {
                return Err(format!("Each argument must be under {MAX_ARG_CHARS} characters, without control characters."));
            }
            secrets.env = resolve_entries(&input.env, &saved.env, "environment variable", valid_env_name)?;
            (command, input.args.clone(), String::new(), Vec::new(), secrets.env.keys().cloned().collect())
        }
        McpTransport::Http | McpTransport::Sse => {
            let url = validate_url(&input.url)?;
            secrets.headers = resolve_entries(&input.headers, &saved.headers, "header", valid_header_name)?;
            if let Some(reserved) = secrets.headers.keys().find(|name| RESERVED_HEADERS.contains(&name.to_ascii_lowercase().as_str())) {
                return Err(format!("WackCode sets the {reserved} header itself. Remove it."));
            }
            if secrets.headers.values().any(|value| value.contains(['\r', '\n'])) {
                return Err("A header value must be a single line.".into());
            }
            if !secrets.headers.is_empty() && url.starts_with("http://") && !is_loopback(&url) {
                return Err("Use an https:// URL to send headers to a remote server, so they aren't sent unencrypted.".into());
            }
            let headers = secrets.headers.keys().cloned().collect();
            (String::new(), Vec::new(), url, headers, Vec::new())
        }
    };

    let record = McpServerRecord {
        id: existing.map_or_else(|| format!("mcp-{}", Uuid::new_v4().simple()), |server| server.id.clone()),
        name,
        enabled: existing.is_none_or(|server| server.enabled),
        transport: input.transport,
        timeout_ms: input.timeout_ms,
        command,
        args,
        url,
        headers,
        env,
        disabled_tools: existing.map(|server| server.disabled_tools.clone()).unwrap_or_default(),
        tools: existing.map(|server| server.tools.clone()).unwrap_or_default(),
    };
    Ok((record, secrets))
}

/// Header or environment entries by name, each with its value: the one typed, or else the one
/// already saved under that name.
fn resolve_entries(
    entries: &[crate::models::McpSecretInput],
    saved: &BTreeMap<String, String>,
    label: &str,
    valid_name: fn(&str) -> bool,
) -> Result<BTreeMap<String, String>, String> {
    if entries.len() > MAX_ENTRIES { return Err(format!("A server can have up to {MAX_ENTRIES} of these.")); }
    let mut resolved = BTreeMap::new();
    for entry in entries {
        let name = entry.name.trim();
        if name.is_empty() && entry.value.as_deref().is_none_or(|value| value.trim().is_empty()) { continue; }
        if name.chars().count() > MAX_ENTRY_NAME_CHARS || !valid_name(name) {
            return Err(format!("\u{201c}{name}\u{201d} isn't a valid {label} name."));
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
    characters.next().is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

/// An HTTP token (RFC 9110 §5.6.2).
fn valid_header_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|character| character.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(character))
}

fn validate_url(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() { return Err("Enter the server's URL.".into()); }
    let parsed = reqwest::Url::parse(value).map_err(|_| "Enter a valid URL, such as https://example.com/mcp.".to_string())?;
    if parsed.scheme() != "https" && parsed.scheme() != "http" { return Err("The URL must start with https:// or http://.".into()); }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("Put credentials in a header, not in the URL.".into());
    }
    Ok(value.to_string())
}

fn is_loopback(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else { return false };
    let Some(host) = parsed.host_str() else { return false };
    let host = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    host == "localhost" || host.ends_with(".localhost") || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// The user's per-tool switches: trimmed, deduplicated and sorted.
pub fn validate_disabled_tools(names: &[String]) -> Result<Vec<String>, String> {
    let mut clean: Vec<String> = names.iter().map(|name| name.trim().to_string()).filter(|name| !name.is_empty()).collect();
    clean.sort();
    clean.dedup();
    if clean.len() > MAX_DISABLED_TOOLS || clean.iter().any(|name| name.chars().count() > MAX_TOOL_NAME_CHARS) {
        return Err("That list of tools is too long.".into());
    }
    Ok(clean)
}

/// Whether two versions of a server reach it the same way (so a tool list still applies).
pub fn same_connection(left: &McpServerRecord, right: &McpServerRecord) -> bool {
    (left.transport, &left.command, &left.args, &left.url, &left.headers, &left.env)
        == (right.transport, &right.command, &right.args, &right.url, &right.headers, &right.env)
}

/// One server as the worker's protocol describes it (`McpServerSpec`), secrets included.
pub fn spec(server: &McpServerRecord, secrets: &McpSecrets) -> Value {
    let mut value = json!({
        "id": server.id,
        "name": server.name,
        "slug": slug(&server.name),
        "transport": server.transport,
        "timeoutMs": server.timeout_ms,
        "disabledTools": server.disabled_tools,
    });
    match server.transport {
        McpTransport::Stdio => {
            value["command"] = json!(server.command);
            value["args"] = json!(server.args);
            value["env"] = json!(secrets.env);
        }
        McpTransport::Http | McpTransport::Sse => {
            value["url"] = json!(server.url);
            value["headers"] = json!(secrets.headers);
        }
    }
    value
}

/// The worker's `mcp` value: every enabled server with its secrets. A disabled server's secrets
/// are never read.
pub fn runtime_payload(servers: &[McpServerRecord], secrets: impl Fn(&str) -> McpSecrets) -> Value {
    Value::Array(servers.iter().filter(|server| server.enabled).map(|server| spec(server, &secrets(&server.id))).collect())
}

#[derive(Deserialize)]
struct ProbeLine {
    ok: bool,
    #[serde(default)]
    tools: Vec<McpToolInfo>,
    #[serde(default)]
    error: Option<String>,
}

/// Connect to one server in a short-lived process (`mcp-probe.js`) and list its tools. The
/// process gets the same environment as a chat worker, with provider keys stripped, and starts
/// in the home folder, since Settings has no project. Its whole process group is killed at the
/// end, so a stdio server can't outlive the test.
pub async fn probe(app: &AppHandle, server: &McpServerRecord, secrets: &McpSecrets) -> Result<Vec<McpToolInfo>, String> {
    let mut command = Command::new(crate::worker::node_executable_path()?);
    command
        .arg(crate::worker::mcp_probe_entry_path(app)?)
        .current_dir(app.path().home_dir().map_err(|error| error.to_string())?)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    crate::shell_env::apply(&mut command).await;
    crate::worker::strip_provider_env(&mut command);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    let mut child = command.spawn().map_err(|error| format!("Could not start the connection test: {error}"))?;
    let pid = child.id();
    let mut stdin = child.stdin.take().ok_or("The connection test has no stdin.")?;
    let mut line = serde_json::to_vec(&json!({ "server": spec(server, secrets) })).map_err(|error| error.to_string())?;
    line.push(b'\n');
    stdin.write_all(&line).await.map_err(|error| format!("Could not start the connection test: {error}"))?;
    drop(stdin);

    // Connecting and listing tools may each take the whole timeout.
    let deadline = Duration::from_millis(server.timeout_ms.saturating_mul(2)).saturating_add(Duration::from_secs(5)).min(MAX_PROBE);
    let output = tokio::time::timeout(deadline, child.wait_with_output()).await;
    if let Some(pid) = pid { let _ = killpg(Pid::from_raw(pid as i32), Signal::SIGKILL); }
    let output = output
        .map_err(|_| format!("The server did not respond within {} ms.", server.timeout_ms))?
        .map_err(|error| format!("The connection test failed: {error}"))?;
    let result = String::from_utf8_lossy(&output.stdout)
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<ProbeLine>(line).ok())
        .ok_or("The connection test stopped without an answer.")?;
    if !result.ok {
        return Err(crate::worker::redact_and_limit(result.error.as_deref().unwrap_or("The server could not be reached.")));
    }
    Ok(result.tools.into_iter().take(MAX_LISTED_TOOLS).map(|tool| McpToolInfo {
        name: tool.name.chars().take(MAX_TOOL_NAME_CHARS).collect(),
        description: tool.description.chars().take(MAX_TOOL_DESCRIPTION_CHARS).collect(),
        read_only: tool.read_only,
    }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::McpSecretInput;

    fn entry(name: &str, value: Option<&str>) -> McpSecretInput {
        McpSecretInput { name: name.into(), value: value.map(str::to_string) }
    }

    fn stdio(name: &str) -> SaveMcpServerInput {
        SaveMcpServerInput {
            id: None,
            name: name.into(),
            transport: McpTransport::Stdio,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            command: "npx".into(),
            args: vec!["-y".into(), "@example/server".into()],
            url: String::new(),
            headers: Vec::new(),
            env: vec![entry("GITHUB_TOKEN", Some("ghp_secretvalue123"))],
        }
    }

    fn http(url: &str, headers: Vec<McpSecretInput>) -> SaveMcpServerInput {
        SaveMcpServerInput {
            transport: McpTransport::Http,
            command: String::new(),
            args: Vec::new(),
            url: url.into(),
            headers,
            env: Vec::new(),
            ..stdio("Remote")
        }
    }

    #[test]
    fn slugs_are_short_lowercase_identifiers() {
        assert_eq!(slug("GitHub"), "github");
        assert_eq!(slug("My  Linear / Jira!"), "my_linear_jira");
        assert_eq!(slug("a-very-long-server-name-indeed"), "a_very_long_server_n");
        assert_eq!(slug("日本"), "server");
    }

    #[test]
    fn a_new_stdio_server_keeps_values_out_of_the_record() {
        let (record, secrets) = validate(&stdio("GitHub"), &[], &McpSecrets::default()).unwrap();
        assert!(record.id.starts_with("mcp-"));
        assert!(record.enabled);
        assert_eq!(record.env, vec!["GITHUB_TOKEN".to_string()]);
        assert_eq!(secrets.env["GITHUB_TOKEN"], "ghp_secretvalue123");
        let json = serde_json::to_string(&record).unwrap();
        assert!(!json.contains("ghp_secretvalue123"));
    }

    #[test]
    fn a_blank_value_keeps_the_saved_one_and_a_missing_one_is_refused() {
        let (record, secrets) = validate(&stdio("GitHub"), &[], &McpSecrets::default()).unwrap();
        let mut edit = stdio("GitHub");
        edit.id = Some(record.id.clone());
        edit.env = vec![entry("GITHUB_TOKEN", Some("  ")), entry("OTHER", None)];
        let error = validate(&edit, std::slice::from_ref(&record), &secrets).unwrap_err();
        assert!(error.contains("OTHER"), "{error}");
        edit.env = vec![entry("GITHUB_TOKEN", None)];
        let (_, kept) = validate(&edit, std::slice::from_ref(&record), &secrets).unwrap();
        assert_eq!(kept.env["GITHUB_TOKEN"], "ghp_secretvalue123");
    }

    #[test]
    fn editing_keeps_the_switch_and_tool_state() {
        let (mut record, secrets) = validate(&stdio("GitHub"), &[], &McpSecrets::default()).unwrap();
        record.enabled = false;
        record.disabled_tools = vec!["delete_repo".into()];
        record.tools = vec![McpToolInfo { name: "delete_repo".into(), description: String::new(), read_only: false }];
        let mut edit = stdio("GitHub renamed");
        edit.id = Some(record.id.clone());
        let (saved, _) = validate(&edit, std::slice::from_ref(&record), &secrets).unwrap();
        assert!(!saved.enabled);
        assert_eq!(saved.disabled_tools, record.disabled_tools);
        assert_eq!(saved.tools, record.tools);
        assert_eq!(saved.name, "GitHub renamed");
    }

    #[test]
    fn names_must_be_distinct_once_slugged() {
        let (record, _) = validate(&stdio("GitHub"), &[], &McpSecrets::default()).unwrap();
        assert!(validate(&stdio("github!"), std::slice::from_ref(&record), &McpSecrets::default()).is_err());
        assert!(validate(&stdio("  "), &[], &McpSecrets::default()).is_err());
    }

    #[test]
    fn timeouts_and_commands_are_checked() {
        let mut input = stdio("A");
        input.timeout_ms = 500;
        assert!(validate(&input, &[], &McpSecrets::default()).is_err());
        let mut input = stdio("A");
        input.command = "npx\nrm -rf".into();
        assert!(validate(&input, &[], &McpSecrets::default()).is_err());
        let mut input = stdio("A");
        input.env = vec![entry("1BAD", Some("x"))];
        assert!(validate(&input, &[], &McpSecrets::default()).is_err());
    }

    #[test]
    fn headers_need_https_unless_the_server_is_local() {
        let bearer = || vec![entry("Authorization", Some("Bearer abcdefgh12345"))];
        assert!(validate(&http("http://mcp.example.com/mcp", bearer()), &[], &McpSecrets::default()).is_err());
        assert!(validate(&http("https://mcp.example.com/mcp", bearer()), &[], &McpSecrets::default()).is_ok());
        assert!(validate(&http("http://127.0.0.1:8080/mcp", bearer()), &[], &McpSecrets::default()).is_ok());
        assert!(validate(&http("http://localhost:8080/mcp", bearer()), &[], &McpSecrets::default()).is_ok());
        assert!(validate(&http("http://[::1]:8080/mcp", bearer()), &[], &McpSecrets::default()).is_ok());
        assert!(validate(&http("http://mcp.example.com/mcp", Vec::new()), &[], &McpSecrets::default()).is_ok());
    }

    #[test]
    fn urls_and_header_names_are_checked() {
        assert!(validate(&http("ftp://example.com", Vec::new()), &[], &McpSecrets::default()).is_err());
        assert!(validate(&http("https://user:pw@example.com/mcp", Vec::new()), &[], &McpSecrets::default()).is_err());
        assert!(validate(&http("https://example.com", vec![entry("Bad Header", Some("x"))]), &[], &McpSecrets::default()).is_err());
        assert!(validate(&http("https://example.com", vec![entry("Content-Type", Some("x"))]), &[], &McpSecrets::default()).is_err());
        assert!(validate(&http("https://example.com", vec![entry("X-Key", Some("a\nb"))]), &[], &McpSecrets::default()).is_err());
        let twice = vec![entry("X-Key", Some("a")), entry("x-key", Some("b"))];
        assert!(validate(&http("https://example.com", twice), &[], &McpSecrets::default()).is_err());
    }

    #[test]
    fn a_disabled_server_sends_nothing_and_reads_no_secret() {
        let (mut record, _) = validate(&stdio("GitHub"), &[], &McpSecrets::default()).unwrap();
        record.enabled = false;
        let payload = runtime_payload(&[record], |_| panic!("no secret may be read"));
        assert_eq!(payload, json!([]));
    }

    #[test]
    fn the_payload_carries_each_enabled_server_with_its_own_secrets() {
        let (stdio_record, stdio_secrets) = validate(&stdio("GitHub"), &[], &McpSecrets::default()).unwrap();
        let (http_record, http_secrets) = validate(
            &http("https://example.com/mcp", vec![entry("Authorization", Some("Bearer abcdefgh12345"))]),
            &[],
            &McpSecrets::default(),
        ).unwrap();
        let servers = [stdio_record.clone(), http_record.clone()];
        let payload = runtime_payload(&servers, |id| if id == stdio_record.id { stdio_secrets.clone() } else { http_secrets.clone() });
        assert_eq!(payload[0]["slug"], "github");
        assert_eq!(payload[0]["transport"], "stdio");
        assert_eq!(payload[0]["env"]["GITHUB_TOKEN"], "ghp_secretvalue123");
        assert!(payload[0].get("headers").is_none());
        assert_eq!(payload[1]["transport"], "http");
        assert_eq!(payload[1]["headers"]["Authorization"], "Bearer abcdefgh12345");
        assert!(payload[1].get("env").is_none());
    }

    #[test]
    fn secrets_round_trip_under_their_own_key() {
        let directory = tempfile::tempdir().unwrap();
        let store = SecretStore::load(directory.path()).unwrap();
        let secrets = McpSecrets { headers: BTreeMap::from([("X-Key".into(), "value12345".into())]), env: BTreeMap::new() };
        store_secrets(&store, "mcp-1", &secrets).unwrap();
        assert_eq!(load_secrets(&store, "mcp-1"), secrets);
        assert!(store.get_optional("mcp:mcp-1").unwrap().is_some());
        store_secrets(&store, "mcp-1", &McpSecrets::default()).unwrap();
        assert!(store.get_optional("mcp:mcp-1").unwrap().is_none());
    }

    #[test]
    fn disabled_tool_names_are_cleaned() {
        let names = vec![" b ".to_string(), "a".into(), "b".into(), String::new()];
        assert_eq!(validate_disabled_tools(&names).unwrap(), vec!["a".to_string(), "b".into()]);
    }
}
