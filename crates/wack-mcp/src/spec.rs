//! One MCP server as a connection needs it, credentials included, and the tools it declares.
//!
//! A [`ServerSpec`] is built by the engine from Settings › MCP servers and `secrets.json` for one
//! run or one Test connection. Its header and environment values are the server's credentials:
//! they go only to that server (headers on requests to its own origin, variables to its own
//! process), and are removed from every error and from the stderr tail ([`ServerSpec::redact`]).

use std::{collections::BTreeMap, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Tool names are `mcp__<slug>__<tool>` within 64 characters, so the slug stays short.
pub const MAX_SLUG_CHARS: usize = 20;
/// Shorter values (flags, ports, "true") are not worth hiding and would mangle ordinary text.
const MIN_SECRET_CHARS: usize = 8;
pub const REDACTED: &str = "[credential redacted]";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    /// A command on this Mac, spoken to over its stdin and stdout.
    #[default]
    Stdio,
    /// Streamable HTTP: one URL that takes POSTs and answers with JSON or an event stream.
    Http,
    /// The older HTTP+SSE transport: an event stream that names where to POST.
    Sse,
}

impl Transport {
    pub fn label(self) -> &'static str {
        match self {
            Transport::Stdio => "stdio",
            Transport::Http => "HTTP",
            Transport::Sse => "SSE",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ServerSpec {
    pub id: String,
    /// The name the user gave it, shown in errors and the transcript.
    pub name: String,
    /// The middle of its tools' names ([`slug`]).
    pub slug: String,
    pub transport: Transport,
    /// How long one request may take, connecting and each call alike.
    pub timeout: Duration,
    pub command: String,
    pub args: Vec<String>,
    /// The server's own environment variables (stdio).
    pub env: BTreeMap<String, String>,
    pub url: String,
    /// Sent with every request to the server (HTTP and SSE).
    pub headers: BTreeMap<String, String>,
    /// The server's own names for the tools the user switched off.
    pub disabled_tools: Vec<String>,
}

impl ServerSpec {
    /// What the connection depends on, hashed: a change means a new connection and a new
    /// catalogue. The name, timeout and tool switches are not part of it. It covers credentials
    /// without keeping them, so it can name a cache file.
    pub fn fingerprint(&self) -> String {
        let key = serde_json::json!([self.transport, self.command, self.args, self.env, self.url, self.headers]);
        hex(&Sha256::digest(key.to_string().as_bytes()))
    }

    /// Header and environment values, for exact-value redaction. For "Bearer <token>" the token
    /// alone is listed too, since an error may quote just that.
    pub fn secret_values(&self) -> Vec<String> {
        let mut values = Vec::new();
        for value in self.headers.values().chain(self.env.values()) {
            values.push(value.clone());
            let mut words = value.split_whitespace();
            if let (Some(_), Some(token), None) = (words.next(), words.next(), words.next()) {
                values.push(token.to_string());
            }
        }
        values.retain(|value| value.chars().count() >= MIN_SECRET_CHARS);
        values
    }

    /// `text` with every credential replaced, longest first so no fragment survives.
    pub fn redact(&self, text: &str) -> String {
        redact_values(text, &self.secret_values())
    }

    pub fn tool_disabled(&self, tool: &str) -> bool {
        self.disabled_tools.iter().any(|name| name == tool)
    }
}

pub fn redact_values(text: &str, secrets: &[String]) -> String {
    let mut secrets: Vec<&String> = secrets.iter().collect();
    secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    let mut result = text.to_string();
    for secret in secrets {
        if !secret.is_empty() {
            result = result.replace(secret.as_str(), REDACTED);
        }
    }
    result
}

/// The middle of a server's tool names: lowercase letters, digits and underscores.
pub fn slug(name: &str) -> String {
    let raw: String = name.chars().map(|character| if character.is_ascii_alphanumeric() { character.to_ascii_lowercase() } else { '_' }).collect();
    let joined = raw.split('_').filter(|part| !part.is_empty()).collect::<Vec<_>>().join("_");
    let short: String = joined.chars().take(MAX_SLUG_CHARS).collect();
    let short = short.trim_end_matches('_');
    if short.is_empty() { "server".into() } else { short.to_string() }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// One tool as a server declares it in `tools/list`: only what WackCode uses, which is also all
/// the catalogue cache keeps.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDecl {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "inputSchema", default = "empty_schema")]
    pub input_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Annotations>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Annotations {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(rename = "readOnlyHint", default, skip_serializing_if = "Option::is_none")]
    pub read_only_hint: Option<bool>,
}

fn empty_schema() -> Value {
    serde_json::json!({ "type": "object" })
}

impl ToolDecl {
    /// The server says the tool changes nothing, so it may run side by side and while planning.
    pub fn read_only(&self) -> bool {
        self.annotations.as_ref().and_then(|annotations| annotations.read_only_hint) == Some(true)
    }

    /// A human title: the tool's own, its annotation's, or its name.
    pub fn label(&self) -> &str {
        self.title
            .as_deref()
            .or_else(|| self.annotations.as_ref().and_then(|annotations| annotations.title.as_deref()))
            .filter(|title| !title.trim().is_empty())
            .unwrap_or(&self.name)
    }

    /// A tool from a `tools/list` reply, or `None` when it has no name.
    pub(crate) fn parse(value: &Value) -> Option<Self> {
        value.get("name")?.as_str()?;
        let mut tool: ToolDecl = serde_json::from_value(value.clone()).ok()?;
        if !tool.input_schema.is_object() {
            tool.input_schema = empty_schema();
        }
        Some(tool)
    }
}

#[cfg(test)]
pub(crate) fn test_spec(name: &str) -> ServerSpec {
    ServerSpec {
        id: format!("mcp-{}", slug(name)),
        name: name.into(),
        slug: slug(name),
        transport: Transport::Stdio,
        timeout: Duration::from_secs(10),
        command: "true".into(),
        args: Vec::new(),
        env: BTreeMap::new(),
        url: String::new(),
        headers: BTreeMap::new(),
        disabled_tools: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_short_lowercase_identifiers() {
        assert_eq!(slug("GitHub"), "github");
        assert_eq!(slug("My  Linear / Jira!"), "my_linear_jira");
        assert_eq!(slug("a-very-long-server-name-indeed"), "a_very_long_server_n");
        assert_eq!(slug("日本"), "server");
    }

    #[test]
    fn secrets_list_values_and_a_bearer_token_alone_skipping_short_ones() {
        let mut spec = test_spec("A");
        spec.headers = BTreeMap::from([("Authorization".into(), "Bearer abcdefgh12345".into()), ("X-Flag".into(), "1".into())]);
        spec.env = BTreeMap::from([("TOKEN".into(), "env-secret-value".into())]);
        assert_eq!(spec.secret_values(), ["Bearer abcdefgh12345", "abcdefgh12345", "env-secret-value"]);
        assert_eq!(spec.redact("sent Bearer abcdefgh12345 and abcdefgh12345"), "sent [credential redacted] and [credential redacted]");
    }

    #[test]
    fn the_fingerprint_follows_the_connection_not_the_name() {
        let spec = test_spec("A");
        let mut renamed = spec.clone();
        renamed.name = "B".into();
        renamed.timeout = Duration::from_secs(1);
        renamed.disabled_tools = vec!["x".into()];
        assert_eq!(spec.fingerprint(), renamed.fingerprint());
        let mut keyed = spec.clone();
        keyed.env.insert("TOKEN".into(), "changed-secret".into());
        assert_ne!(spec.fingerprint(), keyed.fingerprint());
        assert!(!spec.fingerprint().contains("changed"));
        assert_eq!(spec.fingerprint().len(), 64);
    }

    #[test]
    fn declarations_are_read_leniently() {
        let tool = ToolDecl::parse(&serde_json::json!({ "name": "peek", "inputSchema": [], "annotations": { "readOnlyHint": true, "other": 1 }, "extra": 2 })).unwrap();
        assert!(tool.read_only());
        assert!(tool.input_schema.is_object());
        assert_eq!(tool.label(), "peek");
        assert!(ToolDecl::parse(&serde_json::json!({ "description": "nameless" })).is_none());
    }
}
