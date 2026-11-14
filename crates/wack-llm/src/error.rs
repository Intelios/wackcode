//! Errors from a provider, sorted by what the caller should do about them.

use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// Too many requests. Retry after a wait.
    RateLimited,
    /// The provider is busy or failed on its side. Retry.
    Unavailable,
    /// The connection failed or dropped. Retry.
    Network,
    /// The conversation no longer fits the model's context window. Compact, then retry.
    ContextOverflow,
    /// The key was rejected.
    Auth,
    /// The request itself was refused. Retrying the same request cannot help.
    Invalid,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LlmError {
    pub kind: ErrorKind,
    /// A sentence fit to show the user. Never contains the key.
    pub message: String,
    pub retry_after: Option<Duration>,
}

impl LlmError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into(), retry_after: None }
    }

    pub fn retryable(&self) -> bool {
        matches!(self.kind, ErrorKind::RateLimited | ErrorKind::Unavailable | ErrorKind::Network)
    }

    /// Sorts an HTTP failure. `detail` is the provider's own message, when it sent one.
    pub fn from_status(status: u16, detail: &str, retry_after: Option<Duration>) -> Self {
        let detail = detail.trim();
        let kind = if is_context_overflow(status, detail) {
            ErrorKind::ContextOverflow
        } else {
            match status {
                401 | 403 => ErrorKind::Auth,
                408 | 409 | 429 => ErrorKind::RateLimited,
                500..=599 => ErrorKind::Unavailable,
                _ => ErrorKind::Invalid,
            }
        };
        let message = match (kind, detail.is_empty()) {
            (ErrorKind::Auth, true) => "The provider rejected the API key.".to_string(),
            (ErrorKind::Auth, false) => format!("The provider rejected the API key: {detail}"),
            (_, true) => format!("The provider answered with HTTP {status}."),
            (_, false) => format!("The provider answered with HTTP {status}: {detail}"),
        };
        Self { kind, message, retry_after }
    }

    /// Sorts an error reported inside an otherwise successful stream.
    pub fn from_stream(error_type: &str, detail: &str) -> Self {
        let kind = if is_context_overflow(400, detail) {
            ErrorKind::ContextOverflow
        } else {
            match error_type {
                "rate_limit_error" | "rate_limit_exceeded" => ErrorKind::RateLimited,
                "overloaded_error" | "api_error" | "server_error" => ErrorKind::Unavailable,
                "authentication_error" | "permission_error" => ErrorKind::Auth,
                _ => ErrorKind::Invalid,
            }
        };
        let message =
            if detail.is_empty() { "The provider reported an error.".to_string() } else { detail.to_string() };
        Self { kind, message, retry_after: None }
    }
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

fn is_context_overflow(status: u16, detail: &str) -> bool {
    if status == 413 {
        return true;
    }
    let detail = detail.to_ascii_lowercase();
    [
        "prompt is too long",
        "context length",
        "context window",
        "context_length_exceeded",
        "maximum context",
        "too many tokens",
        "input is too long",
        "exceeds the model's maximum",
    ]
    .iter()
    .any(|needle| detail.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_http_failures() {
        assert_eq!(LlmError::from_status(429, "slow down", None).kind, ErrorKind::RateLimited);
        assert_eq!(LlmError::from_status(529, "", None).kind, ErrorKind::Unavailable);
        assert_eq!(LlmError::from_status(401, "", None).kind, ErrorKind::Auth);
        assert_eq!(LlmError::from_status(400, "bad field", None).kind, ErrorKind::Invalid);
        assert_eq!(
            LlmError::from_status(400, "prompt is too long: 250000 tokens > 200000", None).kind,
            ErrorKind::ContextOverflow
        );
        assert_eq!(
            LlmError::from_status(400, "This model's maximum context length is 128000 tokens", None).kind,
            ErrorKind::ContextOverflow
        );
    }

    #[test]
    fn only_transient_failures_retry() {
        assert!(LlmError::new(ErrorKind::Network, "").retryable());
        assert!(!LlmError::new(ErrorKind::ContextOverflow, "").retryable());
        assert!(!LlmError::new(ErrorKind::Invalid, "").retryable());
    }
}
