//! One connection to one MCP server: the handshake, its tool list, and tool calls. Shared by the
//! hub (chats) and Test connection, so both reach a server the same way and fail with the same
//! sentences.
//!
//! Invariants:
//! - Every request has the server's timeout. A request abandoned for any reason (timeout, the
//!   user stopped the run, the caller's future dropped) is forgotten and, except `initialize`,
//!   the server is told with `notifications/cancelled`.
//! - Every error leaves here as a sentence with the server's credentials removed.
//! - Dropping a connection closes it.

use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::{
    spec::{ServerSpec, ToolDecl, Transport},
    transport::{Failure, Groups, Link, Peer},
};

/// The version WackCode asks for, and every version it can speak.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
const SUPPORTED_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
/// A server listing more than this is cut short, as version 2's Settings did.
pub const MAX_TOOLS: usize = 500;
const MAX_PAGES: usize = 50;
const MAX_ERROR_CHARS: usize = 2_000;

pub struct Connection {
    spec: ServerSpec,
    peer: Arc<Peer>,
    link: Arc<Link>,
    tools: Mutex<Vec<ToolDecl>>,
}

impl Connection {
    /// Starts or reaches the server, completes the MCP handshake and lists its tools. A stdio
    /// server starts in `cwd`.
    pub async fn connect(spec: &ServerSpec, cwd: &Path) -> Result<Connection, String> {
        Self::connect_in(spec, cwd, &Groups::default()).await
    }

    /// [`Connection::connect`], with a stdio server's process group recorded in `groups`.
    pub(crate) async fn connect_in(spec: &ServerSpec, cwd: &Path, groups: &Groups) -> Result<Connection, String> {
        let peer = Peer::new();
        let link = Link::open(spec, cwd, &peer, groups).await.map_err(|failure| describe(&failure, spec, None))?;
        let connection = Connection { spec: spec.clone(), peer, link, tools: Mutex::new(Vec::new()) };
        match connection.handshake().await {
            Ok(()) => Ok(connection),
            Err(failure) => {
                let message = connection.describe(&failure);
                connection.close();
                Err(message)
            }
        }
    }

    async fn handshake(&self) -> Result<(), Failure> {
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "WackCode", "version": env!("CARGO_PKG_VERSION") },
        });
        let result = self.request("initialize", params, None, false).await?;
        let version = result.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
        if !SUPPORTED_VERSIONS.contains(&version) {
            return Err(Failure::Said(format!("The server speaks MCP version “{version}”, which WackCode doesn't support.")));
        }
        self.link.set_version(version);
        self.notify("notifications/initialized").await?;
        let tools = self.list_tools().await?;
        *self.tools.lock().unwrap() = tools;
        Ok(())
    }

    async fn list_tools(&self) -> Result<Vec<ToolDecl>, Failure> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let params = match &cursor {
                Some(cursor) => json!({ "cursor": cursor }),
                None => json!({}),
            };
            let result = self.request("tools/list", params, None, true).await?;
            let page = result.get("tools").and_then(Value::as_array).ok_or_else(|| Failure::Said("The server's tool list isn't valid.".into()))?;
            tools.extend(page.iter().filter_map(ToolDecl::parse));
            cursor = result.get("nextCursor").and_then(Value::as_str).filter(|cursor| !cursor.is_empty()).map(str::to_string);
            if cursor.is_none() || tools.len() >= MAX_TOOLS {
                break;
            }
        }
        tools.truncate(MAX_TOOLS);
        Ok(tools)
    }

    /// The server's tools as of the last listing.
    pub fn tools(&self) -> Vec<ToolDecl> {
        self.tools.lock().unwrap().clone()
    }

    /// Lists the tools again if the server said they changed. Returns whether it did.
    pub async fn refresh_if_changed(&self) -> Result<bool, String> {
        if !self.peer.take_tools_changed() {
            return Ok(false);
        }
        let tools = self.list_tools().await.map_err(|failure| self.describe(&failure))?;
        *self.tools.lock().unwrap() = tools;
        Ok(true)
    }

    /// Calls a tool by the server's own name, returning the raw `tools/call` result. Stopping
    /// `cancel` abandons the call and tells the server.
    pub async fn call(&self, tool: &str, arguments: Value, cancel: &CancellationToken) -> Result<Value, String> {
        // The server's stderr explains a failure to start, not a failed call; a process that
        // exited mid-call already says so.
        self.request("tools/call", json!({ "name": tool, "arguments": arguments }), Some(cancel), true)
            .await
            .map_err(|failure| describe(&failure, &self.spec, None))
    }

    /// Why the connection ended, once it has.
    pub fn closed(&self) -> Option<String> {
        self.peer.closed().map(|reason| self.spec.redact(&reason))
    }

    /// Resolves when the connection ends without [`Connection::close`], with the reason.
    pub async fn wait_closed(&self) -> String {
        let reason = self.peer.wait_closed().await;
        self.spec.redact(&reason)
    }

    /// Ends the connection now; a stdio server is asked to stop, then killed.
    pub fn close(&self) {
        self.peer.close("The connection was closed.".into());
        self.link.close();
    }

    /// The stdio server's process group, for quitting.
    pub(crate) fn pgid(&self) -> Option<i32> {
        self.link.pgid()
    }

    /// Lets go without signalling; the caller signals the process group itself.
    pub(crate) fn abandon(&self) {
        self.peer.close("The connection was closed.".into());
        self.link.abandon();
    }

    async fn notify(&self, method: &str) -> Result<(), Failure> {
        let message = json!({ "jsonrpc": "2.0", "method": method });
        Link::send(&self.link, &self.peer, &message, None).await
    }

    async fn request(&self, method: &str, params: Value, cancel: Option<&CancellationToken>, cancellable: bool) -> Result<Value, Failure> {
        if let Some(reason) = self.peer.closed() {
            return Err(Failure::Closed(Some(reason)));
        }
        let (id, answer) = self.peer.register();
        let mut pending = Pending { peer: self.peer.clone(), link: self.link.clone(), id, cancellable, armed: true };
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let exchange = async {
            Link::send(&self.link, &self.peer, &message, Some(id)).await?;
            answer.await.unwrap_or(Err(Failure::Closed(None)))
        };
        let stopped = async {
            match cancel {
                Some(cancel) => cancel.cancelled().await,
                None => std::future::pending().await,
            }
        };
        let result = tokio::select! {
            result = exchange => result,
            () = tokio::time::sleep(self.spec.timeout) => Err(Failure::Timeout),
            () = stopped => Err(Failure::Stopped),
        };
        match &result {
            Err(Failure::Timeout | Failure::Stopped) => {}
            Err(Failure::SessionExpired) => {
                pending.armed = false;
                self.peer.close("The server ended the session.".into());
            }
            _ => pending.armed = false,
        }
        result
    }

    fn describe(&self, failure: &Failure) -> String {
        describe(failure, &self.spec, self.link.stderr_line())
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.close();
    }
}

/// Forgets a request nobody waits for any more, and tells the server it can stop.
struct Pending {
    peer: Arc<Peer>,
    link: Arc<Link>,
    id: u64,
    cancellable: bool,
    armed: bool,
}

impl Drop for Pending {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.peer.forget(self.id);
        if !self.cancellable || self.peer.closed().is_some() {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
        let (peer, link) = (self.peer.clone(), self.link.clone());
        let message = json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": { "requestId": self.id, "reason": "WackCode stopped waiting." } });
        runtime.spawn(async move {
            let _ = Link::send(&link, &peer, &message, None).await;
        });
    }
}

/// A plain sentence for a failed connect or call, ending with the server's last stderr line, with
/// its credentials removed.
pub(crate) fn describe(failure: &Failure, spec: &ServerSpec, stderr_line: Option<String>) -> String {
    let stdio = spec.transport == Transport::Stdio;
    let message = match failure {
        Failure::Stopped => "Stopped.".to_string(),
        Failure::Timeout => format!("It did not respond within {} ms.", spec.timeout.as_millis()),
        Failure::Spawn(error) if error.kind() == std::io::ErrorKind::NotFound => {
            format!("Command not found: {}. Check the command, or give its full path.", spec.command)
        }
        Failure::Spawn(error) if error.kind() == std::io::ErrorKind::PermissionDenied => format!("{} can't be run (permission denied).", spec.command),
        Failure::Spawn(error) => format!("Could not start {}: {error}.", spec.command),
        Failure::Http(status @ (401 | 403)) => {
            format!("The server refused the request (HTTP {status}). If it needs a token, add it as a header, e.g. Authorization: Bearer <token>.")
        }
        Failure::Http(status) => format!("The server answered HTTP {status}."),
        Failure::SessionExpired => "The server ended the session. It reconnects with your next message.".into(),
        Failure::Rpc { code, message } => format!("{} (MCP error {code})", message.trim().trim_end_matches('.')),
        // A closed stdio pipe already carries the server's last words.
        Failure::Closed(Some(reason)) => return finish(spec, reason),
        Failure::Closed(None) if stdio => "The server process exited.".into(),
        Failure::Closed(None) => "The server closed the connection.".into(),
        Failure::Said(sentence) => sentence.clone(),
    };
    let message = match stderr_line {
        Some(line) if stdio && !matches!(failure, Failure::Stopped) => format!("{message} Server output: {line}"),
        _ => message,
    };
    finish(spec, &message)
}

fn finish(spec: &ServerSpec, message: &str) -> String {
    let message = spec.redact(message);
    if message.chars().count() > MAX_ERROR_CHARS {
        format!("{}…", message.chars().take(MAX_ERROR_CHARS).collect::<String>())
    } else {
        message
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::spec::test_spec;

    #[test]
    fn failures_become_plain_sentences_without_secrets() {
        let mut spec = test_spec("Mock");
        spec.command = "uvx".into();
        spec.timeout = Duration::from_millis(2_500);
        spec.env.insert("TOKEN".into(), "secret-value-123".into());
        let missing = Failure::Spawn(std::io::Error::from(std::io::ErrorKind::NotFound));
        assert_eq!(describe(&missing, &spec, None), "Command not found: uvx. Check the command, or give its full path.");
        assert_eq!(describe(&Failure::Timeout, &spec, None), "It did not respond within 2500 ms.");
        assert_eq!(describe(&Failure::Said("boom".into()), &spec, Some("fatal: bad token secret-value-123".into())), "boom Server output: fatal: bad token [credential redacted]");
        assert_eq!(describe(&Failure::Stopped, &spec, Some("noise".into())), "Stopped.");
        spec.transport = Transport::Http;
        assert!(describe(&Failure::Http(401), &spec, None).contains("If it needs a token, add it as a header"));
        assert_eq!(describe(&Failure::Rpc { code: -32602, message: "Unknown tool: x".into() }, &spec, None), "Unknown tool: x (MCP error -32602)");
    }
}
