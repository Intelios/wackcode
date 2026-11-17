//! The MCP client: the user's MCP servers (Settings › MCP servers) as deferred tools named
//! `mcp__<server>__<tool>`. Hand-rolled JSON-RPC 2.0 over stdio, Streamable HTTP and the older
//! HTTP+SSE transport; tools only (no resources or prompts), as in version 2.
//!
//! - `spec`: one server as a connection needs it, and the tools it declares.
//! - `transport` and `client`: one connection, its handshake, tool list and calls.
//! - `hub`: the connections every chat in a project shares, started lazily, closed when idle.
//! - `tool`: the tools a run offers, and the notes that say when a server fails.
//! - `cache`: each server's last tool list, so runs rarely wait for a server to start.
//!
//! The engine (`wack-core`) turns settings and `secrets.json` into [`ServerSpec`]s; nothing here
//! reads the data folder except the catalogue cache it is pointed at. docs/security.md lists what
//! reaches the network and where credentials go.

mod cache;
mod client;
mod convert;
mod hub;
mod spec;
mod tool;
mod transport;

pub use client::{Connection, MAX_TOOLS, PROTOCOL_VERSION};
pub use convert::{MCP_PREFIX, tool_name};
pub use hub::{ConnectionState, Hub, IDLE_AFTER, RETRY_FAILED_AFTER, Server, State};
pub use spec::{Annotations, MAX_SLUG_CHARS, REDACTED, ServerSpec, ToolDecl, Transport, redact_values, slug};
pub use tool::{Assembly, McpFeature, McpTool, NOTE_PREFIX, Starting, assemble};
