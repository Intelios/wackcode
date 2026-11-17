//! The web tools: `web_fetch` reads a public page, and (with a provider set up in Settings › Web)
//! `web_search` searches. Every request goes through `guard.rs`, which keeps them on the public
//! internet; docs/security.md lists these origins.
//!
//! Both tools are deferred: the model finds them with `tool_search` when it needs them.

mod convert;
mod fetch;
pub mod guard;
pub mod search;
mod web_fetch;
mod web_search;

use std::sync::Arc;

use wack_agent::Tool;

pub use fetch::{Fetcher, Policy};
pub use guard::check_public_url;
pub use search::SearchClient;
pub use web_fetch::{WEB_FETCH, WebFetch};
pub use web_search::{WEB_SEARCH, WebSearch};

/// What the web tools are given for one run, read from the engine's settings.
#[derive(Clone, Debug, Default)]
pub struct WebSetup {
    /// Settings › Web's Web Fetch switch.
    pub fetch: bool,
    /// The active search provider, when search is on and it can search. It holds the key.
    pub search: Option<Arc<SearchClient>>,
    /// Read every page through the search provider (it must be able to read pages).
    pub route_fetch: bool,
}

/// The web tools a run offers, in the order the model is shown them.
pub fn tools(setup: &WebSetup) -> Result<Vec<Arc<dyn Tool>>, String> {
    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
    if let Some(search) = &setup.search {
        tools.push(Arc::new(WebSearch::new(search.clone())));
    }
    if setup.fetch {
        let fetch = WebFetch::new(Fetcher::new(Policy::public())?);
        let routed = setup.search.as_ref().filter(|search| setup.route_fetch && search::supports_fetch(search.kind()));
        tools.push(Arc::new(match routed {
            Some(reader) => fetch.through(reader.clone()),
            None => fetch,
        }));
    }
    Ok(tools)
}
