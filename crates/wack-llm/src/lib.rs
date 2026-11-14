//! Talks to model providers.
//!
//! Three wire formats live here (Anthropic Messages, OpenAI Responses, OpenAI-compatible Chat
//! Completions) behind one streaming event type, so nothing above this crate knows which provider
//! it is talking to. This crate has no UI dependency and never reads credentials from disk: a key
//! is handed in by the caller for the lifetime of a request.
