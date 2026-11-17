//! Talks to model providers.
//!
//! Three wire formats live here (Anthropic Messages, OpenAI Responses, OpenAI-compatible Chat
//! Completions) behind one request, one stream of deltas and one response type, so nothing above
//! this crate knows which provider it is talking to. This crate has no UI dependency and never
//! reads credentials from disk: a key is handed in by the caller and lives only in the client that
//! makes the request.

mod anthropic;
mod chat;
mod error;
mod http;
mod model;
mod responses;
mod retry;
/// Server-sent events, shared with the MCP client's HTTP transports.
pub mod sse;
pub mod testing;

use async_trait::async_trait;

pub use error::{ErrorKind, LlmError};
pub use http::HttpProvider;
pub use model::{
    Block, Delta, Effort, Image, Message, Request, Response, Role, StopReason, SystemPart, ToolDef, Usage, WireFormat,
};
pub use retry::{RetryNotice, stream_with_retry};

/// Where deltas go while a response streams. Called from the task that drives the request.
pub type DeltaSink<'a> = &'a (dyn Fn(Delta) + Send + Sync);

/// One way of reaching a model.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Sends `request`, reports text and tool input to `deltas` as it arrives, and returns the
    /// finished response. Dropping the returned future abandons the request.
    async fn stream(&self, request: &Request, deltas: DeltaSink<'_>) -> Result<Response, LlmError>;

    /// Whether this provider discovers deferred tools itself. When it does not, the agent offers
    /// its own search tool instead.
    fn native_tool_search(&self) -> bool {
        false
    }
}
