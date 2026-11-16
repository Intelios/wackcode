//! The web tools: `web_fetch` reads a public page, and (with a provider set up in Settings › Web)
//! `web_search` searches. Every request goes through `guard.rs`, which keeps them on the public
//! internet; docs/security.md lists these origins.
//!
//! Both tools are deferred: the model finds them with `tool_search` when it needs them.

mod convert;
mod fetch;
mod guard;
mod web_fetch;

use std::sync::Arc;

use wack_agent::Tool;

pub use fetch::{Fetcher, Policy};
pub use web_fetch::{WEB_FETCH, WebFetch};

/// What the web tools are given for one run, read from the engine's settings.
#[derive(Clone, Debug, Default)]
pub struct WebSetup {
    /// Settings › Web's Web Fetch switch.
    pub fetch: bool,
}

/// The web tools a run offers, in the order the model is shown them.
pub fn tools(setup: &WebSetup) -> Result<Vec<Arc<dyn Tool>>, String> {
    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
    if setup.fetch {
        tools.push(Arc::new(WebFetch::new(Fetcher::new(Policy::public())?)));
    }
    Ok(tools)
}
